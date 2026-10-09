import type { Platform, StatusUpdate } from "./types";

/** Códigos de `detail` que envía el conector de Twitch (ver `twitch/source.rs`). */
export const TWITCH_DETAIL = { chat: "chat", full: "full", notOwner: "notOwner", connecting: "connecting" } as const;

/**
 * Clave de i18n con la frase, en lenguaje llano, que describe el estado de una plataforma.
 * La UI nunca muestra el estado técnico a secas: siempre dice qué pasa y qué se está recibiendo.
 */
export function statusKey(platform: Platform, s: StatusUpdate): string {
  switch (s.state) {
    case "disconnected":
      return "conn.state.disconnected";
    case "signature_error":
      return "conn.state.signatureError";
    case "reconnecting":
      return "conn.state.reconnecting";
    case "waiting_live":
      if (platform === "twitch") return s.detail === TWITCH_DETAIL.connecting ? "conn.state.twitch.connecting" : "conn.state.twitch.offline";
      return "conn.state.tiktok.waitingLive";
    case "connected":
      if (platform === "twitch") {
        if (s.detail === TWITCH_DETAIL.full) return "conn.state.twitch.full";
        if (s.detail === TWITCH_DETAIL.notOwner) return "conn.state.twitch.notOwner";
        return "conn.state.twitch.chat";
      }
      return "conn.state.tiktok.connected";
  }
}

/** ¿Hay algo en marcha (conectando, conectado o reintentando) en esta plataforma? */
export function isActive(s: StatusUpdate): boolean {
  return s.state !== "disconnected";
}

/** Estado «bueno»: recibiendo eventos. */
export function isHealthy(s: StatusUpdate): boolean {
  return s.state === "connected";
}
