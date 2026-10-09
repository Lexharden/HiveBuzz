// Subconjunto estructural de los mensajes de `tiktok-live-proto` (v3) que usa el
// normalizador. Todo es opcional a propósito: TikTok cambia el protocolo y un
// campo ausente nunca debe tirar la app. Los mensajes reales de la librería son
// asignables a estos tipos.

export interface RawImage {
  urlList?: string[];
}

export interface RawCommon {
  msgId?: string;
  /** Milisegundos (a veces segundos) desde epoch, como string. */
  createTime?: string;
}

export interface RawIdentity {
  isSubscriberOfAnchor?: boolean;
  isFollowerOfAnchor?: boolean;
  isModeratorOfAnchor?: boolean;
}

export interface RawUser {
  id?: string;
  /** El @usuario (uniqueId) viaja en `displayId`. */
  displayId?: string;
  nickname?: string;
  avatarThumb?: RawImage;
  isFollower?: boolean;
  isSubscribe?: boolean;
  userAttr?: { isAdmin?: boolean };
  payGrade?: { level?: number };
  fansClub?: { data?: { level?: number } };
}

export interface RawGiftInfo {
  id?: string;
  name?: string;
  /** 1 = combinable (racha); cualquier otro valor = regalo único. */
  type?: number;
  diamondCount?: number;
  image?: RawImage;
}

export interface RawGiftMessage {
  common?: RawCommon;
  user?: RawUser;
  userIdentity?: RawIdentity;
  giftId?: string;
  groupId?: string;
  repeatCount?: number;
  /** 0/1 en el protocolo. */
  repeatEnd?: number | boolean;
  gift?: RawGiftInfo;
}

export interface RawChatMessage {
  common?: RawCommon;
  user?: RawUser;
  userIdentity?: RawIdentity;
  content?: string;
  emotes?: { emote?: { emoteId?: string; image?: RawImage } }[];
}

export interface RawLikeMessage {
  common?: RawCommon;
  user?: RawUser;
  count?: number;
  /** Total acumulado de la sala (string u number según versión). */
  total?: string | number;
}

export interface RawSocialMessage {
  common?: RawCommon;
  user?: RawUser;
}

export interface RawMemberMessage {
  common?: RawCommon;
  user?: RawUser;
  /** 1 = entró al LIVE. */
  action?: number;
}

export interface RawSubNotifyMessage {
  common?: RawCommon;
  user?: RawUser;
}

export interface RawEmoteMessage {
  common?: RawCommon;
  user?: RawUser;
  userIdentity?: RawIdentity;
  emoteList?: { emoteId?: string; image?: RawImage }[];
}
