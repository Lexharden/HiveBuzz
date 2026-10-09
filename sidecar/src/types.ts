// Esquema normalizado de eventos. Espejo de `src-tauri/src/events.rs`.

export type EventType =
  | "gift"
  | "chat"
  | "like"
  | "follow"
  | "share"
  | "subscribe"
  | "join"
  | "emote"
  | "liveEnd";

export interface LiveUser {
  id: string;
  uniqueId: string;
  nickname: string;
  avatar: string;
  isModerator: boolean;
  isSubscriber: boolean;
  isFollower: boolean;
  teamLevel?: number;
  gifterLevel?: number;
}

export interface LiveGift {
  id: number;
  name: string;
  coins: number;
  count: number;
  streakable: boolean;
  image: string;
}

/** De qué plataforma viene un evento. El sidecar de TikTok no lo envía: Rust lo estampa. */
export type Platform = "tiktok" | "twitch";

export interface LiveEvent {
  /** Id del mensaje en su plataforma (msgId de TikTok), usado para deduplicar. */
  id: string;
  /** Plataforma de origen; si falta, es TikTok. */
  platform?: Platform;
  type: EventType;
  user: LiveUser;
  gift?: LiveGift;
  chat?: { text: string; emotes?: { id: string; image: string }[] };
  like?: { count: number; total: number };
  /** Milisegundos desde epoch. */
  ts: number;
}
