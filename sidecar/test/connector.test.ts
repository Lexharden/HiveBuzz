import { beforeEach, describe, expect, it } from "vitest";
import {
  backoffDelay,
  classifyError,
  Connector,
  DEFAULT_CONFIG,
  type ClientCallbacks,
  type LiveClient,
} from "../src/connector";
import type { SidecarMessage } from "../src/protocol";

// ---- Infraestructura de test ---------------------------------------------------------------

class VirtualClock {
  t = 1_000_000;
  private timers: { at: number; resolve: () => void }[] = [];

  now = () => this.t;

  sleep = (ms: number, signal: AbortSignal): Promise<void> =>
    new Promise((resolve) => {
      if (signal.aborted) return resolve();
      const timer = { at: this.t + ms, resolve };
      this.timers.push(timer);
      signal.addEventListener(
        "abort",
        () => {
          this.timers = this.timers.filter((x) => x !== timer);
          resolve();
        },
        { once: true },
      );
    });

  async settle(): Promise<void> {
    for (let i = 0; i < 5; i++) await new Promise((r) => setImmediate(r));
  }

  async advance(ms: number): Promise<void> {
    const target = this.t + ms;
    await this.settle();
    for (;;) {
      const next = this.timers.filter((x) => x.at <= target).sort((a, b) => a.at - b.at)[0];
      if (!next) break;
      this.timers = this.timers.filter((x) => x !== next);
      this.t = Math.max(this.t, next.at);
      next.resolve();
      await this.settle();
    }
    this.t = target;
    await this.settle();
  }
}

class NamedError extends Error {}
class InvalidUniqueIdError extends NamedError {}
class SignatureRateLimitError extends NamedError {
  retryAfter = 20_000;
}
class UserOfflineError extends NamedError {}

interface FakeClient extends LiveClient {
  cb: ClientCallbacks;
  connected: boolean;
}

interface World {
  clock: VirtualClock;
  out: SidecarMessage[];
  clients: FakeClient[];
  /** Resultado de `isLive()` por llamada; si se agota, devuelve `true`. */
  live: (boolean | Error)[];
  /** Comportamiento de `connect()` por intento; si se agota, conecta bien. */
  connects: (Error | "ok")[];
  /** Si se define, los clientes tienen `sendChat` (es decir, hay sesión de TikTok). */
  chat?: (text: string) => Promise<void>;
  connector: Connector;
}

function world(random = 0): World {
  const clock = new VirtualClock();
  const w = { clock, out: [], clients: [], live: [], connects: [] } as unknown as World;
  w.connector = new Connector({
    factory: (_t, cb) => {
      const c: FakeClient = {
        cb,
        connected: false,
        isLive: async () => {
          const v = w.live.shift() ?? true;
          if (v instanceof Error) throw v;
          return v;
        },
        connect: async () => {
          const v = w.connects.shift() ?? "ok";
          if (v instanceof Error) throw v;
          c.connected = true;
        },
        disconnect: async () => {
          c.connected = false;
        },
        ...(w.chat ? { sendChat: (text: string) => (w.chat as (t: string) => Promise<void>)(text) } : {}),
      };
      w.clients.push(c);
      return c;
    },
    emit: (m) => w.out.push(m),
    now: clock.now,
    sleep: clock.sleep,
    random: () => random,
    config: { sendTimeoutMs: 40 },
  });
  return w;
}

const states = (w: World) => w.out.flatMap((m) => (m.kind === "status" ? [m.state] : []));
const events = (w: World) => w.out.flatMap((m) => (m.kind === "event" ? [m.event] : []));
const statusMsgs = (w: World) => w.out.filter((m): m is Extract<SidecarMessage, { kind: "status" }> => m.kind === "status");
const lastClient = (w: World): FakeClient => {
  const c = w.clients.at(-1);
  if (!c) throw new Error("no hay cliente");
  return c;
};
const chat = (id: string, text = "hola") => ({
  t: "chat" as const,
  msg: { common: { msgId: id }, user: { id: "1", displayId: "ana", nickname: "Ana" }, content: text },
});

let w: World;
beforeEach(() => {
  w = world();
});

// ---- Pruebas -------------------------------------------------------------------------------

describe("funciones puras", () => {
  it("backoffDelay crece exponencialmente y se topa en 60 s", () => {
    const d = (a: number, r: number) => backoffDelay(a, DEFAULT_CONFIG, r);
    expect([0, 1, 2, 3].map((a) => d(a, 1))).toEqual([1_000, 2_000, 4_000, 8_000]);
    expect(d(10, 1)).toBe(60_000);
    expect(d(1000, 1)).toBe(60_000);
  });

  it("el jitter queda entre cap/2 y cap", () => {
    expect(backoffDelay(3, DEFAULT_CONFIG, 0)).toBe(4_000);
    expect(backoffDelay(3, DEFAULT_CONFIG, 0.5)).toBe(6_000);
  });

  it("classifyError", () => {
    expect(classifyError(new InvalidUniqueIdError("x"))).toBe("fatal");
    expect(classifyError(new UserOfflineError("x"))).toBe("offline");
    expect(classifyError(new SignatureRateLimitError("x"))).toBe("signature");
    expect(classifyError(new Error("red"))).toBe("transient");
    expect(classifyError("texto")).toBe("transient");
  });
});

describe("conexión", () => {
  it("espera al LIVE consultando cada 30 s y luego conecta", async () => {
    w.live = [false, false, true];
    await w.connector.connect({ uniqueId: "@ana" });
    await w.clock.advance(0);
    expect(states(w)).toEqual(["waiting_live"]);
    await w.clock.advance(30_000);
    expect(w.connector.state).toBe("waiting_live");
    await w.clock.advance(30_000);
    expect(w.connector.state).toBe("connected");
    await w.connector.disconnect();
  });

  it("emite eventos normalizados y vacía los likes tras la ventana", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    const { cb } = lastClient(w);
    cb.onMessage(chat("c1"));
    cb.onMessage({ t: "like", msg: { common: { msgId: "l1" }, user: { id: "2", displayId: "bob" }, count: 4, total: "9" } });
    cb.onMessage({ t: "like", msg: { common: { msgId: "l2" }, user: { id: "2", displayId: "bob" }, count: 6, total: "15" } });
    expect(events(w).map((e) => e.type)).toEqual(["chat"]);
    await w.clock.advance(1_250);
    const like = events(w).find((e) => e.type === "like");
    expect(like?.like).toEqual({ count: 10, total: 15 });
    await w.connector.disconnect();
  });

  it("ignora mensajes duplicados que llegan al reconectar (el LRU sobrevive)", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    lastClient(w).cb.onMessage(chat("dup"));
    lastClient(w).cb.onDisconnected({ code: 1006 });
    await w.clock.advance(1_000);
    expect(w.clients).toHaveLength(2);
    lastClient(w).cb.onMessage(chat("dup")); // TikTok reenvía
    lastClient(w).cb.onMessage(chat("nuevo"));
    expect(events(w).map((e) => e.id)).toEqual(["dup", "nuevo"]);
    await w.connector.disconnect();
  });
});

describe("recuperación", () => {
  it("si el socket se cierra pasa a reconnecting y vuelve a conectar solo", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    expect(w.connector.state).toBe("connected");
    lastClient(w).cb.onDisconnected({ code: 1006, reason: "boom" });
    await w.clock.advance(0);
    expect(w.connector.state).toBe("reconnecting");
    const retry = statusMsgs(w).find((m) => m.retryInMs !== undefined);
    expect(retry).toMatchObject({ state: "reconnecting", attempt: 1, retryInMs: 500 });
    await w.clock.advance(500);
    expect(w.connector.state).toBe("connected");
    expect(w.clients).toHaveLength(2);
    await w.connector.disconnect();
  });

  it("los fallos consecutivos aplican backoff exponencial y se reinicia al conectar", async () => {
    w.connects = [new Error("a"), new Error("b"), new Error("c")];
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    const delays = () => statusMsgs(w).flatMap((m) => (m.retryInMs !== undefined ? [m.retryInMs] : []));
    expect(delays()).toEqual([500]); // cap 1000 con random 0 → 500
    await w.clock.advance(500);
    expect(delays()).toEqual([500, 1_000]);
    await w.clock.advance(1_000);
    expect(delays()).toEqual([500, 1_000, 2_000]);
    await w.clock.advance(2_000);
    expect(w.connector.state).toBe("connected");
    // Tras conectar bien, el siguiente fallo vuelve a empezar desde el principio.
    lastClient(w).cb.onDisconnected({});
    await w.clock.advance(0);
    expect(delays().at(-1)).toBe(500);
    await w.connector.disconnect();
  });

  it("un error de firma muestra signature_error y respeta retry-after", async () => {
    w.connects = [new SignatureRateLimitError("rate limited")];
    await w.connector.connect({ uniqueId: "ana", eulerApiKey: "k" });
    await w.clock.advance(0);
    expect(w.connector.state).toBe("signature_error");
    const retry = statusMsgs(w).find((m) => m.retryInMs !== undefined);
    expect(retry?.retryInMs).toBe(20_000); // retryAfter supera al backoff (500 ms)
    await w.clock.advance(19_999);
    expect(w.connector.state).toBe("signature_error");
    await w.clock.advance(1);
    expect(w.connector.state).toBe("connected");
    await w.connector.disconnect();
  });

  it("un @usuario inválido es fatal: no reintenta", async () => {
    w.live = [new InvalidUniqueIdError("usuario inválido")];
    await w.connector.connect({ uniqueId: "???" });
    await w.clock.advance(120_000);
    expect(w.connector.state).toBe("disconnected");
    expect(w.clients).toHaveLength(1);
    expect(statusMsgs(w).at(-1)?.detail).toContain("usuario inválido");
  });

  it("UserOffline al conectar vuelve a esperar el LIVE sin backoff", async () => {
    w.connects = [new UserOfflineError("offline")];
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    expect(w.connector.state).toBe("waiting_live");
    await w.clock.advance(30_000);
    expect(w.connector.state).toBe("connected");
    await w.connector.disconnect();
  });

  it("heartbeat: sin actividad durante 45 s reconecta", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    await w.clock.advance(44_000);
    expect(w.connector.state).toBe("connected");
    lastClient(w).cb.onActivity(); // un frame de vida reinicia el contador
    await w.clock.advance(44_000);
    expect(w.connector.state).toBe("connected");
    await w.clock.advance(1_000); // 45 s desde la última señal
    expect(w.connector.state).toBe("reconnecting");
    expect(w.clients[0]?.connected).toBe(false);
    await w.clock.advance(500);
    expect(w.connector.state).toBe("connected");
    expect(w.clients).toHaveLength(2);
    await w.connector.disconnect();
  });

  it("los callbacks de una sesión vieja no afectan a la nueva", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    const old = lastClient(w);
    old.cb.onDisconnected({});
    await w.clock.advance(500);
    expect(w.clients).toHaveLength(2);
    old.cb.onDisconnected({}); // rezagado
    old.cb.onMessage(chat("fantasma"));
    await w.clock.advance(0);
    expect(w.connector.state).toBe("connected");
    expect(events(w)).toEqual([]);
    await w.connector.disconnect();
  });
});

describe("escribir en el chat (bot)", () => {
  const results = (w: World) => w.out.flatMap((m) => (m.kind === "chatResult" ? [m] : []));

  it("sin conexión responde con un error claro (nunca se queda callado)", async () => {
    await w.connector.sendChat("r1", "hola");
    expect(results(w)).toEqual([{ kind: "chatResult", requestId: "r1", ok: false, error: "no hay conexión con el LIVE" }]);
  });

  it("conectado pero sin sesión de TikTok: explica qué falta", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    await w.connector.sendChat("r1", "hola");
    expect(results(w)[0]).toMatchObject({ ok: false });
    expect(results(w)[0]?.error).toContain("iniciar sesión");
    await w.connector.disconnect();
  });

  it("envía el texto limpio (una línea, recortado a 150 caracteres sin partir emojis)", async () => {
    const sent: string[] = [];
    w.chat = async (t) => void sent.push(t);
    await w.connector.connect({ uniqueId: "ana", session: { sessionId: "s", ttTargetIdc: "t" } });
    await w.clock.advance(0);
    await w.connector.sendChat("r1", "  hola\r\nmundo  ");
    await w.connector.sendChat("r2", "🔥".repeat(200));
    expect(sent[0]).toBe("hola mundo");
    expect([...(sent[1] ?? "")]).toHaveLength(150);
    expect(sent[1]).toBe("🔥".repeat(150));
    expect(results(w).map((r) => [r.requestId, r.ok])).toEqual([["r1", true], ["r2", true]]);
    await w.connector.disconnect();
  });

  it("rechaza mensajes vacíos sin tocar TikTok", async () => {
    let calls = 0;
    w.chat = async () => void calls++;
    await w.connector.connect({ uniqueId: "ana", session: { sessionId: "s", ttTargetIdc: "t" } });
    await w.clock.advance(0);
    await w.connector.sendChat("r1", " \r\n ");
    expect(calls).toBe(0);
    expect(results(w)[0]).toMatchObject({ ok: false, error: "mensaje vacío" });
    await w.connector.disconnect();
  });

  it("un fallo de TikTok se informa con su motivo", async () => {
    w.chat = async () => {
      throw new Error("rate limited");
    };
    await w.connector.connect({ uniqueId: "ana", session: { sessionId: "s", ttTargetIdc: "t" } });
    await w.clock.advance(0);
    await w.connector.sendChat("r1", "hola");
    expect(results(w)[0]).toMatchObject({ ok: false, error: "rate limited" });
    await w.connector.disconnect();
  });

  it("si TikTok no responde a tiempo, se informa en vez de colgarse", async () => {
    w.chat = () => new Promise(() => undefined);
    await w.connector.connect({ uniqueId: "ana", session: { sessionId: "s", ttTargetIdc: "t" } });
    await w.clock.advance(0);
    await w.connector.sendChat("r1", "hola");
    expect(results(w)[0]).toMatchObject({ ok: false });
    expect(results(w)[0]?.error).toContain("tardó demasiado");
    await w.connector.disconnect();
  });

  it("tras desconectar vuelve a no haber conexión", async () => {
    w.chat = async () => undefined;
    await w.connector.connect({ uniqueId: "ana", session: { sessionId: "s", ttTargetIdc: "t" } });
    await w.clock.advance(0);
    await w.connector.disconnect();
    await w.connector.sendChat("r1", "hola");
    expect(results(w)[0]).toMatchObject({ ok: false, error: "no hay conexión con el LIVE" });
  });
});

describe("espectadores", () => {
  const viewers = (w: World) => w.out.flatMap((m) => (m.kind === "viewers" ? [m.count] : []));

  it("emite el conteo solo si cambió y como mucho una vez por segundo", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    const { cb } = lastClient(w);
    cb.onViewers(10);
    cb.onViewers(10); // igual: se omite
    cb.onViewers(11); // demasiado pronto: se omite
    expect(viewers(w)).toEqual([10]);
    await w.clock.advance(1_000);
    cb.onViewers(11);
    cb.onViewers(12.9); // se trunca
    expect(viewers(w)).toEqual([10, 11]);
    await w.clock.advance(1_000);
    cb.onViewers(12);
    expect(viewers(w)).toEqual([10, 11, 12]);
    await w.connector.disconnect();
  });

  it("ignora valores inválidos y cuenta como señal de vida para el heartbeat", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    const { cb } = lastClient(w);
    cb.onViewers(Number.NaN);
    cb.onViewers(-3);
    cb.onViewers(Number.POSITIVE_INFINITY);
    expect(viewers(w)).toEqual([]);
    await w.clock.advance(44_000);
    cb.onViewers(5);
    await w.clock.advance(44_000);
    expect(w.connector.state).toBe("connected");
    await w.connector.disconnect();
  });

  it("la memoria del último valor se reinicia en cada sesión", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    lastClient(w).cb.onViewers(7);
    lastClient(w).cb.onDisconnected({});
    await w.clock.advance(500);
    lastClient(w).cb.onViewers(7);
    expect(viewers(w)).toEqual([7, 7]);
    await w.connector.disconnect();
  });
});

describe("fin del LIVE", () => {
  it("emite liveEnd, vacía lo pendiente y vuelve a esperar", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    lastClient(w).cb.onMessage({ t: "like", msg: { common: { msgId: "l" }, user: { id: "2", displayId: "bob" }, count: 3, total: "3" } });
    lastClient(w).cb.onStreamEnd();
    await w.clock.advance(0);
    expect(events(w).map((e) => e.type)).toEqual(["like", "liveEnd"]);
    expect(w.connector.state).toBe("waiting_live");
    await w.clock.advance(30_000);
    expect(w.connector.state).toBe("connected"); // el streamer volvió
    await w.connector.disconnect();
  });
});

describe("disconnect", () => {
  it("detiene todo y deja el estado en disconnected", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    await w.connector.disconnect();
    expect(w.connector.state).toBe("disconnected");
    expect(w.clients[0]?.connected).toBe(false);
    await w.clock.advance(120_000);
    expect(w.clients).toHaveLength(1);
  });

  it("también cancela una espera de LIVE o un backoff en curso", async () => {
    w.live = [false];
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    await w.connector.disconnect();
    expect(w.connector.state).toBe("disconnected");

    w.connects = [new Error("x")];
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    expect(w.connector.state).toBe("reconnecting");
    await w.connector.disconnect();
    expect(w.connector.state).toBe("disconnected");
  });

  it("connect() mientras hay una sesión la reemplaza sin dejar clientes vivos", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    await w.connector.connect({ uniqueId: "bob" });
    await w.clock.advance(0);
    expect(w.clients.filter((c) => c.connected)).toHaveLength(1);
    await w.connector.disconnect();
  });

  it("dos connect() sin esperar dejan un solo supervisor", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    const a = w.connector.connect({ uniqueId: "bob" });
    const b = w.connector.connect({ uniqueId: "carla" });
    await Promise.all([a, b]);
    await w.clock.advance(0);
    expect(w.clients.filter((c) => c.connected)).toHaveLength(1);
    await w.connector.disconnect();
    await w.clock.advance(0);
    expect(w.clients.filter((c) => c.connected)).toHaveLength(0);
    expect(w.connector.state).toBe("disconnected");
  });

  it("los errores de la librería ({ info, exception }) se registran legibles", async () => {
    await w.connector.connect({ uniqueId: "ana" });
    await w.clock.advance(0);
    lastClient(w).cb.onError({ info: "fallo de firma", exception: new Error("HTTP 429") });
    const logs = w.out.flatMap((m) => (m.kind === "log" ? [m.message] : []));
    expect(logs.some((l) => l.includes("fallo de firma: HTTP 429"))).toBe(true);
    await w.connector.disconnect();
  });
});
