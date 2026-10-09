// Protocolo NDJSON con Rust. Espejo de `src-tauri/src/source/protocol.rs`.

import type { LiveEvent } from "./types";

export type ConnectionState =
  | "disconnected"
  | "waiting_live"
  | "connected"
  | "signature_error"
  | "reconnecting";

export type LogLevel = "debug" | "info" | "warn" | "error";

/** Sidecar → Rust (stdout). */
export type SidecarMessage =
  | { kind: "ready" }
  | { kind: "event"; event: LiveEvent }
  | {
      kind: "status";
      state: ConnectionState;
      detail?: string;
      attempt?: number;
      retryInMs?: number;
    }
  | { kind: "log"; level: LogLevel; message: string }
  /** Espectadores conectados ahora mismo (limitado a ~1 mensaje por segundo y solo si cambió). */
  | { kind: "viewers"; count: number }
  /** Resultado de un `sendChat` (se corresponde por `requestId`). */
  | { kind: "chatResult"; requestId: string; ok: boolean; error?: string };

/** Sesión de TikTok del usuario (solo hace falta para que el bot escriba en el chat). */
export interface TikTokSession {
  sessionId: string;
  ttTargetIdc: string;
}

/** Rust → sidecar (stdin). */
export type SidecarCommand =
  | { cmd: "connect"; uniqueId: string; eulerApiKey?: string; session?: TikTokSession }
  | { cmd: "disconnect" }
  | { cmd: "sendChat"; requestId: string; text: string }
  | { cmd: "shutdown" };

export function encode(msg: SidecarMessage): string {
  return JSON.stringify(msg) + "\n";
}

/** Una sesión a medias (falta alguna cookie) se descarta: sería inútil y confunde al diagnosticar. */
function parseSession(raw: unknown): { session: TikTokSession } | Record<string, never> {
  if (typeof raw !== "object" || raw === null) return {};
  const s = raw as Record<string, unknown>;
  if (typeof s.sessionId !== "string" || s.sessionId === "" || typeof s.ttTargetIdc !== "string" || s.ttTargetIdc === "") return {};
  return { session: { sessionId: s.sessionId, ttTargetIdc: s.ttTargetIdc } };
}

/** Devuelve `null` si la línea no es un comando válido (el llamador lo registra). */
export function parseCommand(line: string): SidecarCommand | null {
  let raw: unknown;
  try {
    raw = JSON.parse(line);
  } catch {
    return null;
  }
  if (typeof raw !== "object" || raw === null) return null;
  const o = raw as Record<string, unknown>;
  switch (o.cmd) {
    case "connect":
      if (typeof o.uniqueId !== "string" || o.uniqueId.trim() === "") return null;
      return {
        cmd: "connect",
        uniqueId: o.uniqueId,
        ...(typeof o.eulerApiKey === "string" ? { eulerApiKey: o.eulerApiKey } : {}),
        ...parseSession(o.session),
      };
    case "disconnect":
      return { cmd: "disconnect" };
    case "sendChat":
      if (typeof o.requestId !== "string" || o.requestId === "" || typeof o.text !== "string") return null;
      return { cmd: "sendChat", requestId: o.requestId, text: o.text };
    case "shutdown":
      return { cmd: "shutdown" };
    default:
      return null;
  }
}
