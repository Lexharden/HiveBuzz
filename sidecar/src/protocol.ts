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
  | { kind: "viewers"; count: number };

/** Rust → sidecar (stdin). */
export type SidecarCommand =
  | { cmd: "connect"; uniqueId: string; eulerApiKey?: string }
  | { cmd: "disconnect" }
  | { cmd: "shutdown" };

export function encode(msg: SidecarMessage): string {
  return JSON.stringify(msg) + "\n";
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
      };
    case "disconnect":
      return { cmd: "disconnect" };
    case "shutdown":
      return { cmd: "shutdown" };
    default:
      return null;
  }
}
