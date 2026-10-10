// Supervisor de la conexión: espera al LIVE, conecta, vigila la salud y se recupera solo.
//
// No conoce `tiktok-live-connector`: habla con un `LiveClient` inyectado (ver `tiktok.ts`),
// lo que permite probar toda la lógica de resiliencia con un cliente falso y un reloj virtual.

import { Normalizer } from "./normalizer";
import type { ConnectionState, LogLevel, SidecarMessage, TikTokSession } from "./protocol";
import type {
  RawChatMessage,
  RawEmoteMessage,
  RawGiftMessage,
  RawLikeMessage,
  RawMemberMessage,
  RawSocialMessage,
  RawSubNotifyMessage,
} from "./raw";

export type RawInput =
  | { t: "gift"; msg: RawGiftMessage }
  | { t: "chat"; msg: RawChatMessage }
  | { t: "like"; msg: RawLikeMessage }
  | { t: "follow" | "share"; msg: RawSocialMessage }
  | { t: "member"; msg: RawMemberMessage }
  | { t: "subNotify"; msg: RawSubNotifyMessage }
  | { t: "emote"; msg: RawEmoteMessage };

export interface ClientCallbacks {
  onMessage(input: RawInput): void;
  /** Cualquier señal de vida del socket (incluidos frames que no producen eventos). */
  onActivity(): void;
  /** Número de espectadores conectados (de los mensajes `roomUser`). */
  onViewers(count: number): void;
  onStreamEnd(): void;
  onDisconnected(info: { code?: number; reason?: string }): void;
  onError(err: unknown): void;
  /** Mensaje de un tipo que no manejamos, o que falló al decodificarse. */
  onUnknown(what: string, detail?: unknown): void;
}

export interface LiveClient {
  isLive(): Promise<boolean>;
  connect(): Promise<void>;
  disconnect(): Promise<void>;
  /** Solo existe si hay sesión de TikTok (sin ella el bot no puede escribir). */
  sendChat?(text: string): Promise<void>;
}

export interface ConnectTarget {
  uniqueId: string;
  eulerApiKey?: string;
  session?: TikTokSession;
}

/** Máximo de caracteres de un mensaje de chat de TikTok. */
export const MAX_CHAT_CHARS = 150;

export type ClientFactory = (target: ConnectTarget, cb: ClientCallbacks) => LiveClient;

export interface ConnectorConfig {
  backoffBaseMs: number;
  backoffMaxMs: number;
  /** Sin ninguna señal durante este tiempo estando conectado → se reconecta. */
  heartbeatMs: number;
  /** Cada cuánto se consulta si el streamer ya abrió su LIVE. */
  liveCheckMs: number;
  /** Periodo del bucle de mantenimiento (vaciado de likes/rachas y chequeo de heartbeat). */
  tickMs: number;
  /** Tiempo máximo esperando a que TikTok acepte un mensaje del bot. */
  sendTimeoutMs: number;
}

const VIEWERS_MIN_INTERVAL_MS = 1_000;

export const DEFAULT_CONFIG: ConnectorConfig = {
  backoffBaseMs: 1_000,
  backoffMaxMs: 60_000,
  heartbeatMs: 45_000,
  liveCheckMs: 30_000,
  tickMs: 250,
  sendTimeoutMs: 15_000,
};

export interface ConnectorDeps {
  factory: ClientFactory;
  emit: (msg: SidecarMessage) => void;
  now?: () => number;
  /** Debe resolverse (no rechazar) al abortarse la señal. */
  sleep?: (ms: number, signal: AbortSignal) => Promise<void>;
  random?: () => number;
  config?: Partial<ConnectorConfig>;
}

type EndReason = "stream_end" | "disconnected" | "heartbeat";

/** Backoff exponencial con jitter: ~[cap/2, cap], con cap = min(max, base·2^intento). */
export function backoffDelay(attempt: number, cfg: ConnectorConfig, random: number): number {
  const cap = Math.min(cfg.backoffMaxMs, cfg.backoffBaseMs * 2 ** Math.min(attempt, 30));
  return Math.round(cap / 2 + (random * cap) / 2);
}

export type ErrorKind = "fatal" | "offline" | "signature" | "transient";

/** Clasifica por nombre de clase para no depender de la librería. */
export function classifyError(err: unknown): ErrorKind {
  const name = err instanceof Error ? err.constructor.name : "";
  if (name === "InvalidUniqueIdError") return "fatal";
  if (name === "UserOfflineError") return "offline";
  if (
    name === "SignAPIError" ||
    name === "SignatureRateLimitError" ||
    name === "SignatureMissingTokensError" ||
    name === "PremiumFeatureError" ||
    name === "AuthenticatedWebSocketConnectionError"
  ) {
    return "signature";
  }
  return "transient";
}

class Session {
  lastActivity: number;
  endReason: EndReason | null = null;
  readonly ended: Promise<EndReason>;
  private resolveEnded!: (r: EndReason) => void;

  constructor(now: number) {
    this.lastActivity = now;
    this.ended = new Promise((res) => {
      this.resolveEnded = res;
    });
  }

  end(reason: EndReason): void {
    if (this.endReason) return;
    this.endReason = reason;
    this.resolveEnded(reason);
  }
}

export class Connector {
  private readonly cfg: ConnectorConfig;
  private readonly now: () => number;
  private readonly sleep: (ms: number, signal: AbortSignal) => Promise<void>;
  private readonly random: () => number;
  private normalizer: Normalizer;
  private normalizerFor = "";
  private abort: AbortController | null = null;
  private running: Promise<void> | null = null;
  private current: Session | null = null;
  state: ConnectionState = "disconnected";
  private lastDetail: string | undefined;
  private lastViewers = { count: -1, at: 0 };
  /** El cliente mientras está conectado (con él se puede escribir en el chat). */
  private connectedClient: LiveClient | null = null;
  /** `connect`/`disconnect` se ejecutan de uno en uno: dos `connect` seguidos no crean dos supervisores. */
  private op: Promise<void> = Promise.resolve();

  constructor(private readonly deps: ConnectorDeps) {
    this.cfg = { ...DEFAULT_CONFIG, ...deps.config };
    this.now = deps.now ?? Date.now;
    this.sleep = deps.sleep ?? defaultSleep;
    this.random = deps.random ?? Math.random;
    this.normalizer = this.newNormalizer();
  }

  /** Inicia (o reinicia) la conexión al LIVE de `target.uniqueId`. */
  connect(target: ConnectTarget): Promise<void> {
    return this.serial(() => this.start(target));
  }

  /** Detiene la conexión y espera a que el supervisor termine. */
  disconnect(): Promise<void> {
    return this.serial(() => this.stop());
  }

  private serial(fn: () => Promise<void>): Promise<void> {
    const next = this.op.then(fn, fn);
    this.op = next.catch(() => {});
    return next;
  }

  private async start(target: ConnectTarget): Promise<void> {
    await this.stop();
    const uniqueId = target.uniqueId.trim().replace(/^@/, "");
    // El LRU de deduplicación se conserva entre reconexiones al mismo streamer.
    if (this.normalizerFor !== uniqueId) {
      this.normalizer = this.newNormalizer();
      this.normalizerFor = uniqueId;
    }
    const abort = new AbortController();
    this.abort = abort;
    this.running = this.run({ ...target, uniqueId }, abort.signal).catch((err: unknown) => {
      this.log("error", `el supervisor terminó con error inesperado: ${errMsg(err)}`);
      this.setState("disconnected", { detail: errMsg(err) });
    });
  }

  /**
   * Escribe en el chat del LIVE. Siempre responde con un `chatResult` (éxito o motivo del fallo),
   * así quien lo pidió nunca se queda esperando.
   */
  async sendChat(requestId: string, text: string): Promise<void> {
    const reply = (ok: boolean, error?: string) =>
      this.deps.emit({ kind: "chatResult", requestId, ok, ...(error !== undefined ? { error } : {}) });
    const client = this.connectedClient;
    if (!client) return reply(false, "no hay conexión con el LIVE");
    if (!client.sendChat) return reply(false, "falta iniciar sesión en TikTok (y tener una API key de Euler)");
    // Una sola línea y dentro del límite de TikTok (sin partir un emoji por la mitad).
    const clean = [...text.replace(/[\r\n]+/g, " ").trim()].slice(0, MAX_CHAT_CHARS).join("");
    if (clean === "") return reply(false, "mensaje vacío");
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([
        client.sendChat(clean),
        new Promise<never>((_, rej) => {
          timer = setTimeout(() => rej(new Error("TikTok tardó demasiado en responder")), this.cfg.sendTimeoutMs);
        }),
      ]);
      return reply(true);
    } catch (err) {
      return reply(false, errMsg(err));
    } finally {
      clearTimeout(timer);
    }
  }

  private async stop(): Promise<void> {
    this.abort?.abort();
    const running = this.running;
    this.abort = null;
    this.running = null;
    if (running) await running;
  }

  // ---- Bucle principal ------------------------------------------------------------------

  private async run(target: ConnectTarget, signal: AbortSignal): Promise<void> {
    let attempt = 0;
    try {
      while (!signal.aborted) {
        const session = new Session(this.now());
        this.lastViewers = { count: -1, at: 0 };
        this.current = session;
        const client = this.deps.factory(target, this.callbacks(session));
        let retryAfterMs = 0;

        try {
          if (!(await this.waitUntilLive(client, signal))) break;
          await client.connect();
          if (signal.aborted) {
            await safeDisconnect(client);
            break;
          }
          attempt = 0;
          session.lastActivity = this.now();
          this.setState("connected");
          this.connectedClient = client;

          const reason = await this.monitor(session, signal);
          this.connectedClient = null;
          await safeDisconnect(client);
          this.emitEvents(this.normalizer.flushAll(this.now()));
          if (reason === "aborted") break;
          if (reason === "stream_end") {
            this.emitEvents(this.normalizer.liveEnd(this.now()));
            this.setState("waiting_live", { detail: "el LIVE terminó" });
            await this.sleep(this.cfg.liveCheckMs, signal);
            continue;
          }
          this.setState("reconnecting", { detail: reason === "heartbeat" ? "sin actividad" : "conexión cerrada" });
        } catch (err) {
          this.connectedClient = null;
          await safeDisconnect(client);
          this.emitEvents(this.normalizer.flushAll(this.now()));
          if (signal.aborted) break;
          const kind = classifyError(err);
          this.log("warn", `fallo de conexión (${kind}): ${errMsg(err)}`);
          if (kind === "fatal") {
            this.setState("disconnected", { detail: errMsg(err) });
            return;
          }
          if (kind === "offline") {
            this.setState("waiting_live", { detail: "el streamer no está en vivo" });
            await this.sleep(this.cfg.liveCheckMs, signal);
            continue;
          }
          retryAfterMs = Number((err as { retryAfter?: unknown }).retryAfter) || 0;
          this.setState(kind === "signature" ? "signature_error" : "reconnecting", { detail: errMsg(err) });
        }

        // Reintento con backoff exponencial + jitter (respeta el retry-after de la firma).
        const delay = Math.max(backoffDelay(attempt, this.cfg, this.random()), retryAfterMs);
        attempt += 1;
        this.setState(this.state, { detail: this.lastDetail, attempt, retryInMs: delay });
        await this.sleep(delay, signal);
      }
    } finally {
      this.current = null;
      this.connectedClient = null;
      this.emitEvents(this.normalizer.flushAll(this.now()));
      if (this.state !== "disconnected") this.setState("disconnected");
    }
  }

  /** Devuelve `false` si se abortó mientras esperaba. */
  private async waitUntilLive(client: LiveClient, signal: AbortSignal): Promise<boolean> {
    while (!signal.aborted) {
      if (await client.isLive()) return true;
      this.setState("waiting_live", { detail: "esperando a que el streamer abra su LIVE" });
      await this.sleep(this.cfg.liveCheckMs, signal);
    }
    return false;
  }

  /** Mantenimiento mientras hay sesión: vacía likes/rachas y vigila el heartbeat. */
  private async monitor(session: Session, signal: AbortSignal): Promise<EndReason | "aborted"> {
    for (;;) {
      if (signal.aborted) return "aborted";
      if (session.endReason) return session.endReason;
      await Promise.race([session.ended, this.sleep(this.cfg.tickMs, signal)]);
      if (signal.aborted) return "aborted";
      this.emitEvents(this.normalizer.flush(this.now()));
      if (session.endReason) return session.endReason;
      if (this.now() - session.lastActivity >= this.cfg.heartbeatMs) {
        this.log("warn", `sin actividad durante ${this.cfg.heartbeatMs} ms; se reconecta`);
        return "heartbeat";
      }
    }
  }

  // ---- Callbacks del cliente ------------------------------------------------------------

  private callbacks(session: Session): ClientCallbacks {
    // Las sesiones viejas no pueden afectar a la actual.
    const live = () => this.current === session && !session.endReason;
    return {
      onMessage: (input) => {
        if (!live()) return;
        session.lastActivity = this.now();
        this.emitEvents(this.dispatch(input));
      },
      onActivity: () => {
        if (live()) session.lastActivity = this.now();
      },
      onViewers: (count) => {
        if (!live()) return;
        session.lastActivity = this.now();
        this.emitViewers(count);
      },
      onStreamEnd: () => {
        if (this.current === session) session.end("stream_end");
      },
      onDisconnected: (info) => {
        if (this.current !== session) return;
        this.log("info", `socket cerrado (${info.code ?? "?"}${info.reason ? `: ${info.reason}` : ""})`);
        session.end("disconnected");
      },
      onError: (err) => this.log("warn", `error de la librería: ${errMsg(err)}`),
      onUnknown: (what, detail) => this.normalizer.unknown(what, detail),
    };
  }

  private dispatch(input: RawInput) {
    const now = this.now();
    switch (input.t) {
      case "gift":
        return this.normalizer.gift(input.msg, now);
      case "chat":
        return this.normalizer.chat(input.msg, now);
      case "like":
        return this.normalizer.like(input.msg, now);
      case "follow":
      case "share":
        return this.normalizer.social(input.t, input.msg, now);
      case "member":
        return this.normalizer.member(input.msg, now);
      case "subNotify":
        return this.normalizer.subNotify(input.msg, now);
      case "emote":
        return this.normalizer.emote(input.msg, now);
    }
  }

  // ---- Salida ---------------------------------------------------------------------------

  /** Solo si cambió y como mucho una vez por segundo (TikTok lo manda cada pocos segundos). */
  private emitViewers(count: number): void {
    if (!Number.isFinite(count) || count < 0) return;
    const n = Math.trunc(count);
    const now = this.now();
    if (n === this.lastViewers.count || now - this.lastViewers.at < VIEWERS_MIN_INTERVAL_MS) return;
    this.lastViewers = { count: n, at: now };
    this.deps.emit({ kind: "viewers", count: n });
  }

  private emitEvents(events: ReturnType<Normalizer["flush"]>): void {
    for (const event of events) this.deps.emit({ kind: "event", event });
  }

  private setState(
    state: ConnectionState,
    extra: { detail?: string | undefined; attempt?: number; retryInMs?: number } = {},
  ): void {
    this.state = state;
    const { detail, attempt, retryInMs } = extra;
    this.lastDetail = detail;
    this.deps.emit({
      kind: "status",
      state,
      ...(detail !== undefined ? { detail } : {}),
      ...(attempt !== undefined ? { attempt } : {}),
      ...(retryInMs !== undefined ? { retryInMs } : {}),
    });
  }

  private log(level: LogLevel, message: string): void {
    this.deps.emit({ kind: "log", level, message });
  }

  private newNormalizer(): Normalizer {
    return new Normalizer({ log: (level, message) => this.log(level, message) });
  }
}

async function safeDisconnect(client: LiveClient): Promise<void> {
  try {
    await client.disconnect();
  } catch {
    // Ya estaba caído; nada que hacer.
  }
}

function errMsg(err: unknown): string {
  if (err instanceof Error) return err.message;
  // La librería emite `error` como `{ info, exception }`.
  if (err && typeof err === "object") {
    const { info, exception } = err as { info?: unknown; exception?: unknown };
    if (info !== undefined || exception !== undefined) {
      return [info, exception].filter((v) => v !== undefined).map(errMsg).join(": ");
    }
    try {
      return JSON.stringify(err).slice(0, 300);
    } catch {
      // Objeto circular: cae al String de abajo.
    }
  }
  return String(err);
}

function defaultSleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted) return resolve();
    const done = () => {
      clearTimeout(timer);
      signal.removeEventListener("abort", done);
      resolve();
    };
    const timer = setTimeout(done, ms);
    signal.addEventListener("abort", done, { once: true });
  });
}
