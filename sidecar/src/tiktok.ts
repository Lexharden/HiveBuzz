// Adaptador de `tiktok-live-connector` (v2) a la interfaz `LiveClient`.
// Es la única pieza que conoce la librería: cambiarla o cambiar el proveedor de firma
// (hoy Euler Stream) solo toca este archivo.

import { ControlAction, ControlEvent, PremiumFeatureError, TikTokLiveConnection, WebcastEvent } from "tiktok-live-connector";
import type { ClientCallbacks, ClientFactory, ConnectTarget, LiveClient } from "./connector";

/** Euler Stream puede exigir un plan de pago para enviar chat; se muestra su respuesta real, sin darla por segura. */
export const chatRefusedMessage = (detail: string): string =>
  `Euler Stream rechazó enviar el mensaje (escribir en el chat puede requerir un plan de pago; revisa tu plan): ${detail}`.slice(0, 400);

export const createTikTokClient: ClientFactory = (target: ConnectTarget, cb: ClientCallbacks): LiveClient => {
  const common = {
    // Firma: tier gratuito de Euler Stream; la API key propia es opcional.
    ...(target.eulerApiKey ? { signApiKey: target.eulerApiKey } : {}),
    processInitialData: true,
    // Ya comprobamos `isLive()` nosotros; si el LIVE cae justo al conectar, la librería lanza
    // UserOfflineError y el supervisor vuelve a esperar.
    fetchRoomInfoOnConnect: true,
  };
  // Sesión de TikTok (opcional): solo hace falta para que el bot escriba en el chat. Son dos
  // construcciones explícitas porque las opciones de la librería son una unión discriminada.
  const connection = target.session
    ? new TikTokLiveConnection(target.uniqueId, {
        ...common,
        session: { cookie: { type: "cookie", value: { sessionId: target.session.sessionId, ttTargetIdc: target.session.ttTargetIdc } } },
        authenticateWs: true,
      })
    : new TikTokLiveConnection(target.uniqueId, common);

  // Eventos que normalizamos.
  connection.on(WebcastEvent.GIFT, (msg) => cb.onMessage({ t: "gift", msg }));
  connection.on(WebcastEvent.CHAT, (msg) => cb.onMessage({ t: "chat", msg }));
  connection.on(WebcastEvent.LIKE, (msg) => cb.onMessage({ t: "like", msg }));
  connection.on(WebcastEvent.FOLLOW, (msg) => cb.onMessage({ t: "follow", msg }));
  connection.on(WebcastEvent.SHARE, (msg) => cb.onMessage({ t: "share", msg }));
  connection.on(WebcastEvent.MEMBER, (msg) => cb.onMessage({ t: "member", msg }));
  connection.on(WebcastEvent.SUB_NOTIFY, (msg) => cb.onMessage({ t: "subNotify", msg }));
  connection.on(WebcastEvent.EMOTE, (msg) => cb.onMessage({ t: "emote", msg }));

  // Espectadores conectados: `total` del mensaje roomUser (llega como string o número).
  connection.on(WebcastEvent.ROOM_USER, (msg) => cb.onViewers(Number(msg.total)));

  connection.on(WebcastEvent.STREAM_END, ({ action }) => {
    if (action === ControlAction.CONTROL_ACTION_STREAM_ENDED) cb.onStreamEnd();
  });

  // Control de la conexión.
  connection.on(ControlEvent.DISCONNECTED, (info) => cb.onDisconnected(info));
  connection.on(ControlEvent.ERROR, (err) => cb.onError(err));
  // Cualquier frame del socket cuenta como señal de vida (heartbeat).
  connection.on(ControlEvent.WEBSOCKET_DATA, () => cb.onActivity());

  // Regla 5: registrar (una vez por tipo) lo que no manejamos y todo fallo de decodificación.
  const handled = new Set<string>([
    "WebcastGiftMessage",
    "WebcastChatMessage",
    "WebcastLikeMessage",
    "WebcastSocialMessage",
    "WebcastMemberMessage",
    "WebcastSubNotifyMessage",
    "WebcastEmoteChatMessage",
    "WebcastControlMessage",
    "WebcastRoomUserSeqMessage",
  ]);
  const reported = new Set<string>();
  connection.on(ControlEvent.DECODED_DATA, (type, event) => {
    const decodeError = (event as { decodeError?: unknown } | null)?.decodeError;
    if (decodeError) {
      cb.onUnknown(`fallo al decodificar ${type}`, decodeError);
      return;
    }
    if (!handled.has(type) && !reported.has(type)) {
      reported.add(type);
      cb.onUnknown(`tipo sin manejar: ${type}`);
    }
  });

  return {
    isLive: () => connection.fetchIsLive(),
    connect: async () => {
      await connection.connect();
    },
    disconnect: () => connection.disconnect(),
    // Sin sesión no hay `sendChat`: el conector responde con un error claro en vez de intentarlo.
    ...(target.session
      ? {
          sendChat: async (text: string) => {
            try {
              await connection.sendMessage(text);
            } catch (err) {
              // Euler Stream envía los mensajes desde su nube y lo cobra: sin plan de pago responde 401/403.
              if (err instanceof PremiumFeatureError) throw new Error(chatRefusedMessage(err.message));
              throw err;
            }
          },
        }
      : {}),
  };
};
