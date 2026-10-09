// Normalizador: mensajes crudos de TikTok → `LiveEvent`.
//
// Reglas críticas del normalizador:
//  1. Deduplicación por msgId con LRU (TikTok reenvía mensajes al reconectar).
//  2. Regalos combinables (gift.type === 1): solo se emiten con repeatEnd, con el conteo final.
//  3. Regalos NO combinables (Galaxy, Whale, Universe…): se emiten al primer mensaje.
//     NUNCA se espera repeatEnd para ellos.
//  4. Likes en ráfagas: se agregan por usuario en ventanas de `likeWindowMs`.
//  5. Mensajes desconocidos o malformados: se registran en log, jamás lanzan.
//
// El tiempo entra siempre por parámetro (`now`) para que sea testeable sin timers.

import { LruSet } from "./lru";
import type { LogLevel } from "./protocol";
import type {
  RawChatMessage,
  RawCommon,
  RawEmoteMessage,
  RawGiftMessage,
  RawIdentity,
  RawLikeMessage,
  RawMemberMessage,
  RawSocialMessage,
  RawSubNotifyMessage,
  RawUser,
} from "./raw";
import type { LiveEvent, LiveUser } from "./types";

export type LogFn = (level: LogLevel, message: string) => void;

export interface NormalizerOptions {
  log: LogFn;
  /** Tamaño del LRU de deduplicación. */
  dedupeCapacity?: number;
  /** Ventana de agregación de likes por usuario. */
  likeWindowMs?: number;
  /**
   * Una racha sin `repeatEnd` ni nuevos mensajes durante este tiempo se emite igualmente
   * con el último conteo visto (red de seguridad: evita perder regalos si se pierde el cierre).
   */
  streakTimeoutMs?: number;
}

/** Valor de `gift.type` que identifica a los regalos combinables. */
const STREAKABLE_GIFT_TYPE = 1;
/** `MemberMessageAction.JOINED` */
const MEMBER_ACTION_JOINED = 1;

interface StreakState {
  user: LiveUser;
  giftId: number;
  name: string;
  unitCoins: number;
  image: string;
  count: number;
  lastMsgId: string;
  lastSeen: number;
}

interface LikeWindow {
  user: LiveUser;
  count: number;
  total: number;
  startedAt: number;
  lastMsgId: string;
}

export class Normalizer {
  private readonly log: LogFn;
  private readonly seenIds: LruSet;
  private readonly likeWindowMs: number;
  private readonly streakTimeoutMs: number;
  private readonly streaks = new Map<string, StreakState>();
  private readonly likes = new Map<string, LikeWindow>();
  private seq = 0;

  constructor(opts: NormalizerOptions) {
    this.log = opts.log;
    this.seenIds = new LruSet(opts.dedupeCapacity ?? 10_000);
    this.likeWindowMs = opts.likeWindowMs ?? 1_000;
    this.streakTimeoutMs = opts.streakTimeoutMs ?? 10_000;
  }

  // ---- Handlers por tipo de mensaje -----------------------------------------------------

  gift(msg: RawGiftMessage, now: number): LiveEvent[] {
    return this.guard("gift", () => {
      const msgId = this.msgId(msg.common);
      if (this.isDuplicate(msgId)) return [];
      const user = this.user(msg.user, msg.userIdentity);
      if (!user) return this.drop("gift", "sin usuario");

      const giftId = Number(msg.gift?.id ?? msg.giftId ?? 0);
      const name = msg.gift?.name ?? "";
      const unitCoins = Math.max(0, Number(msg.gift?.diamondCount ?? 0));
      const image = msg.gift?.image?.urlList?.[0] ?? "";
      const repeatCount = Math.max(1, Math.trunc(Number(msg.repeatCount ?? 1)) || 1);
      const streakable = msg.gift?.type === STREAKABLE_GIFT_TYPE;

      // Regla 3: los no combinables salen YA, sin mirar repeatEnd.
      if (!streakable) {
        return [this.giftEvent(msgId, msg.common, now, user, giftId, name, unitCoins, repeatCount, false, image)];
      }

      // Regla 2: combinables. Se acumulan hasta repeatEnd.
      const key = `${user.id}:${giftId}:${msg.groupId ?? ""}`;
      const prev = this.streaks.get(key);
      const count = Math.max(repeatCount, prev?.count ?? 0);
      if (!this.isRepeatEnd(msg.repeatEnd)) {
        this.streaks.set(key, { user, giftId, name, unitCoins, image, count, lastMsgId: msgId, lastSeen: now });
        return [];
      }
      this.streaks.delete(key);
      return [this.giftEvent(msgId, msg.common, now, user, giftId, name, unitCoins, count, true, image)];
    });
  }

  chat(msg: RawChatMessage, now: number): LiveEvent[] {
    return this.guard("chat", () => {
      const msgId = this.msgId(msg.common);
      if (this.isDuplicate(msgId)) return [];
      const user = this.user(msg.user, msg.userIdentity);
      if (!user) return this.drop("chat", "sin usuario");

      const emotes = (msg.emotes ?? []).flatMap((e) => {
        const id = e.emote?.emoteId;
        return id ? [{ id, image: e.emote?.image?.urlList?.[0] ?? "" }] : [];
      });
      return [
        {
          id: msgId,
          type: "chat",
          user,
          chat: { text: msg.content ?? "", ...(emotes.length > 0 ? { emotes } : {}) },
          ts: this.ts(msg.common, now),
        },
      ];
    });
  }

  like(msg: RawLikeMessage, now: number): LiveEvent[] {
    return this.guard("like", () => {
      const msgId = this.msgId(msg.common);
      if (this.isDuplicate(msgId)) return [];
      const user = this.user(msg.user);
      if (!user) return this.drop("like", "sin usuario");

      const count = Math.max(0, Math.trunc(Number(msg.count ?? 0)) || 0);
      const total = Math.max(0, Number(msg.total ?? 0) || 0);
      const win = this.likes.get(user.id);
      if (win) {
        win.count += count;
        win.total = Math.max(win.total, total);
        win.lastMsgId = msgId;
      } else {
        this.likes.set(user.id, { user, count, total, startedAt: now, lastMsgId: msgId });
      }
      return [];
    });
  }

  social(kind: "follow" | "share", msg: RawSocialMessage, now: number): LiveEvent[] {
    return this.guard(kind, () => {
      const msgId = this.msgId(msg.common);
      if (this.isDuplicate(msgId)) return [];
      const user = this.user(msg.user);
      if (!user) return this.drop(kind, "sin usuario");
      return [{ id: msgId, type: kind, user, ts: this.ts(msg.common, now) }];
    });
  }

  member(msg: RawMemberMessage, now: number): LiveEvent[] {
    return this.guard("member", () => {
      // Solo las entradas al LIVE. Las suscripciones llegan por `subNotify`
      // (procesar ambas las contaría dos veces).
      if (msg.action !== MEMBER_ACTION_JOINED) return [];
      const msgId = this.msgId(msg.common);
      if (this.isDuplicate(msgId)) return [];
      const user = this.user(msg.user);
      if (!user) return this.drop("member", "sin usuario");
      return [{ id: msgId, type: "join", user, ts: this.ts(msg.common, now) }];
    });
  }

  subNotify(msg: RawSubNotifyMessage, now: number): LiveEvent[] {
    return this.guard("subNotify", () => {
      const msgId = this.msgId(msg.common);
      if (this.isDuplicate(msgId)) return [];
      const user = this.user(msg.user);
      if (!user) return this.drop("subNotify", "sin usuario");
      return [{ id: msgId, type: "subscribe", user: { ...user, isSubscriber: true }, ts: this.ts(msg.common, now) }];
    });
  }

  emote(msg: RawEmoteMessage, now: number): LiveEvent[] {
    return this.guard("emote", () => {
      const msgId = this.msgId(msg.common);
      if (this.isDuplicate(msgId)) return [];
      const user = this.user(msg.user, msg.userIdentity);
      if (!user) return this.drop("emote", "sin usuario");
      const emotes = (msg.emoteList ?? []).flatMap((e) =>
        e.emoteId ? [{ id: e.emoteId, image: e.image?.urlList?.[0] ?? "" }] : [],
      );
      return [{ id: msgId, type: "emote", user, chat: { text: "", emotes }, ts: this.ts(msg.common, now) }];
    });
  }

  /** El LIVE terminó: se vacía todo lo pendiente y se emite `liveEnd`. */
  liveEnd(now: number): LiveEvent[] {
    const pending = this.flushAll(now);
    const ghost: LiveUser = {
      id: "",
      uniqueId: "",
      nickname: "",
      avatar: "",
      isModerator: false,
      isSubscriber: false,
      isFollower: false,
    };
    return [...pending, { id: `liveEnd:${now}`, type: "liveEnd", user: ghost, ts: now }];
  }

  // ---- Vaciado de lo pendiente ----------------------------------------------------------

  /** Emite las ventanas de likes vencidas y las rachas estancadas. Llamar periódicamente. */
  flush(now: number): LiveEvent[] {
    const out: LiveEvent[] = [];
    for (const [key, win] of this.likes) {
      if (now - win.startedAt >= this.likeWindowMs) {
        this.likes.delete(key);
        out.push(this.likeEvent(win, now));
      }
    }
    for (const [key, st] of this.streaks) {
      if (now - st.lastSeen >= this.streakTimeoutMs) {
        this.streaks.delete(key);
        this.log("warn", `racha sin repeatEnd tras ${this.streakTimeoutMs} ms; se emite con el último conteo (${st.name} x${st.count})`);
        out.push(this.streakEvent(st, now));
      }
    }
    return out;
  }

  /** Emite todo lo pendiente sin esperar plazos (desconexión o fin del LIVE). */
  flushAll(now: number): LiveEvent[] {
    const out: LiveEvent[] = [];
    for (const win of this.likes.values()) out.push(this.likeEvent(win, now));
    for (const st of this.streaks.values()) out.push(this.streakEvent(st, now));
    this.likes.clear();
    this.streaks.clear();
    return out;
  }

  /** Registra un mensaje que la librería no pudo decodificar o que no conocemos. */
  unknown(what: string, detail?: unknown): void {
    this.log("warn", `mensaje no manejado: ${what}${detail === undefined ? "" : ` (${describe(detail)})`}`);
  }

  // ---- Internos -------------------------------------------------------------------------

  /** Regla 5: ningún mensaje malformado puede tirar el proceso. */
  private guard(what: string, fn: () => LiveEvent[]): LiveEvent[] {
    try {
      return fn();
    } catch (err) {
      this.log("error", `fallo normalizando ${what}: ${describe(err)}`);
      return [];
    }
  }

  private drop(what: string, why: string): LiveEvent[] {
    this.log("warn", `${what} descartado: ${why}`);
    return [];
  }

  private msgId(common: RawCommon | undefined): string {
    const id = common?.msgId;
    return id && id !== "0" ? id : `gen:${++this.seq}`;
  }

  /** Los ids generados nunca se repiten, así que no se deduplican. */
  private isDuplicate(msgId: string): boolean {
    return !msgId.startsWith("gen:") && this.seenIds.seen(msgId);
  }

  private isRepeatEnd(v: number | boolean | undefined): boolean {
    return v === true || (typeof v === "number" && v !== 0);
  }

  private ts(common: RawCommon | undefined, now: number): number {
    const n = Number(common?.createTime);
    if (!Number.isFinite(n) || n <= 0) return now;
    return n < 1e12 ? n * 1000 : n; // a veces viene en segundos
  }

  private user(u: RawUser | undefined, identity?: RawIdentity): LiveUser | null {
    if (!u) return null;
    const id = u.id ?? "";
    const uniqueId = u.displayId ?? "";
    if (!id && !uniqueId) return null;
    const gifterLevel = u.payGrade?.level;
    const teamLevel = u.fansClub?.data?.level;
    return {
      id: id || uniqueId,
      uniqueId,
      nickname: u.nickname ?? uniqueId,
      avatar: u.avatarThumb?.urlList?.[0] ?? "",
      isModerator: identity?.isModeratorOfAnchor ?? u.userAttr?.isAdmin ?? false,
      isSubscriber: identity?.isSubscriberOfAnchor ?? u.isSubscribe ?? false,
      isFollower: identity?.isFollowerOfAnchor ?? u.isFollower ?? false,
      ...(teamLevel && teamLevel > 0 ? { teamLevel } : {}),
      ...(gifterLevel && gifterLevel > 0 ? { gifterLevel } : {}),
    };
  }

  private giftEvent(
    id: string,
    common: RawCommon | undefined,
    now: number,
    user: LiveUser,
    giftId: number,
    name: string,
    unitCoins: number,
    count: number,
    streakable: boolean,
    image: string,
  ): LiveEvent {
    return {
      id,
      type: "gift",
      user,
      // `coins` es el valor TOTAL del regalo (monedas por unidad × cantidad).
      gift: { id: giftId, name, coins: unitCoins * count, count, streakable, image },
      ts: this.ts(common, now),
    };
  }

  private streakEvent(st: StreakState, now: number): LiveEvent {
    return this.giftEvent(st.lastMsgId, undefined, now, st.user, st.giftId, st.name, st.unitCoins, st.count, true, st.image);
  }

  private likeEvent(win: LikeWindow, now: number): LiveEvent {
    return {
      id: `like:${win.lastMsgId}`,
      type: "like",
      user: win.user,
      like: { count: win.count, total: win.total },
      ts: now,
    };
  }
}

function describe(v: unknown): string {
  if (v instanceof Error) return v.message;
  if (typeof v === "string") return v;
  try {
    return JSON.stringify(v)?.slice(0, 200) ?? String(v);
  } catch {
    return String(v);
  }
}
