import { beforeEach, describe, expect, it } from "vitest";
import { LruSet } from "../src/lru";
import { Normalizer } from "../src/normalizer";
import type { RawGiftMessage, RawLikeMessage, RawUser } from "../src/raw";

const logs: string[] = [];
let n: Normalizer;

const ana: RawUser = { id: "1", displayId: "ana", nickname: "Ana" };
const bob: RawUser = { id: "2", displayId: "bob", nickname: "Bob" };

function gift(over: Partial<RawGiftMessage> & { msgId?: string; type?: number } = {}): RawGiftMessage {
  const { msgId = "m1", type = 1, ...rest } = over;
  return {
    common: { msgId },
    user: ana,
    giftId: "5655",
    groupId: "g1",
    repeatCount: 1,
    repeatEnd: 0,
    gift: { id: "5655", name: "Rose", type, diamondCount: 1 },
    ...rest,
  };
}

function like(msgId: string, count: number, total: number, user = ana): RawLikeMessage {
  return { common: { msgId }, user, count, total: String(total) };
}

beforeEach(() => {
  logs.length = 0;
  n = new Normalizer({ log: (l, m) => logs.push(`${l}: ${m}`) });
});

describe("LruSet", () => {
  it("expulsa la entrada más antigua al superar la capacidad", () => {
    const lru = new LruSet(2);
    expect(lru.seen("a")).toBe(false);
    expect(lru.seen("b")).toBe(false);
    expect(lru.seen("c")).toBe(false); // expulsa "a"
    expect(lru.size).toBe(2);
    expect(lru.seen("a")).toBe(false);
  });

  it("consultar una clave la marca como reciente", () => {
    const lru = new LruSet(2);
    lru.seen("a");
    lru.seen("b");
    lru.seen("a"); // "b" pasa a ser la más antigua
    lru.seen("c"); // expulsa "b"
    expect(lru.seen("a")).toBe(true);
    expect(lru.seen("b")).toBe(false);
  });
});

describe("deduplicación", () => {
  it("ignora un chat reenviado con el mismo msgId", () => {
    const msg = { common: { msgId: "c1" }, user: ana, content: "hola" };
    expect(n.chat(msg, 0)).toHaveLength(1);
    expect(n.chat(msg, 1)).toHaveLength(0);
  });

  it("ignora un regalo no combinable reenviado", () => {
    expect(n.gift(gift({ msgId: "g", type: 2 }), 0)).toHaveLength(1);
    expect(n.gift(gift({ msgId: "g", type: 2 }), 1)).toHaveLength(0);
  });

  it("no deduplica mensajes sin msgId", () => {
    const msg = { user: ana, content: "x" };
    expect(n.chat(msg, 0)).toHaveLength(1);
    expect(n.chat(msg, 0)).toHaveLength(1);
  });

  it("el LRU de 10 000 olvida lo más antiguo", () => {
    const small = new Normalizer({ log: () => {}, dedupeCapacity: 2 });
    const c = (id: string) => ({ common: { msgId: id }, user: ana, content: "x" });
    small.chat(c("a"), 0);
    small.chat(c("b"), 0);
    small.chat(c("c"), 0); // expulsa "a"
    expect(small.chat(c("a"), 0)).toHaveLength(1);
  });
});

describe("regalos combinables (streaks)", () => {
  it("no emite mientras la racha sigue y emite una vez con el conteo final", () => {
    expect(n.gift(gift({ msgId: "1", repeatCount: 1 }), 0)).toEqual([]);
    expect(n.gift(gift({ msgId: "2", repeatCount: 2 }), 100)).toEqual([]);
    expect(n.gift(gift({ msgId: "3", repeatCount: 5 }), 200)).toEqual([]);
    const out = n.gift(gift({ msgId: "4", repeatCount: 5, repeatEnd: 1 }), 300);
    expect(out).toHaveLength(1);
    expect(out[0]?.gift).toMatchObject({ name: "Rose", count: 5, coins: 5, streakable: true });
    expect(out[0]?.id).toBe("4");
  });

  it("repeatEnd sin mensajes previos emite igualmente", () => {
    const out = n.gift(gift({ repeatCount: 3, repeatEnd: 1 }), 0);
    expect(out[0]?.gift?.count).toBe(3);
  });

  it("acepta repeatEnd booleano", () => {
    const out = n.gift(gift({ repeatCount: 2, repeatEnd: true }), 0);
    expect(out).toHaveLength(1);
  });

  it("rachas de usuarios o grupos distintos no se mezclan", () => {
    n.gift(gift({ msgId: "a", repeatCount: 4 }), 0);
    n.gift(gift({ msgId: "b", repeatCount: 9, user: bob }), 0);
    const a = n.gift(gift({ msgId: "c", repeatCount: 4, repeatEnd: 1 }), 0);
    const b = n.gift(gift({ msgId: "d", repeatCount: 9, repeatEnd: 1, user: bob }), 0);
    expect(a[0]?.gift?.count).toBe(4);
    expect(b[0]?.gift?.count).toBe(9);
    expect(b[0]?.user.uniqueId).toBe("bob");
  });

  it("si el cierre trae menos conteo que lo ya visto, conserva el máximo", () => {
    n.gift(gift({ msgId: "1", repeatCount: 7 }), 0);
    const out = n.gift(gift({ msgId: "2", repeatCount: 1, repeatEnd: 1 }), 1);
    expect(out[0]?.gift?.count).toBe(7);
  });

  it("una racha sin repeatEnd se emite tras el timeout (red de seguridad)", () => {
    n.gift(gift({ repeatCount: 6 }), 0);
    expect(n.flush(9_999)).toEqual([]);
    const out = n.flush(10_000);
    expect(out).toHaveLength(1);
    expect(out[0]?.gift?.count).toBe(6);
    expect(n.flush(20_000)).toEqual([]);
  });

  it("flushAll emite las rachas abiertas (desconexión)", () => {
    n.gift(gift({ repeatCount: 2 }), 0);
    expect(n.flushAll(1)).toHaveLength(1);
  });
});

describe("regalos NO combinables (Galaxy, Whale, Universe…)", () => {
  it("se emiten al primer mensaje aunque repeatEnd sea 0", () => {
    const out = n.gift(
      gift({ type: 2, repeatEnd: 0, gift: { id: "9", name: "Galaxy", type: 2, diamondCount: 1000 } }),
      0,
    );
    expect(out).toHaveLength(1);
    expect(out[0]?.gift).toMatchObject({ name: "Galaxy", coins: 1000, count: 1, streakable: false });
  });

  it("no se quedan retenidos como racha pendiente", () => {
    n.gift(gift({ type: 2 }), 0);
    expect(n.flushAll(1)).toEqual([]);
  });

  it("un regalo sin información de tipo se emite en vez de perderse", () => {
    const out = n.gift({ common: { msgId: "x" }, user: ana, giftId: "1", repeatCount: 1 }, 0);
    expect(out).toHaveLength(1);
  });

  it("coins es el valor total: unidad × cantidad", () => {
    const out = n.gift(gift({ type: 2, repeatCount: 3, gift: { id: "1", name: "X", type: 2, diamondCount: 50 } }), 0);
    expect(out[0]?.gift).toMatchObject({ coins: 150, count: 3 });
  });
});

describe("likes agregados", () => {
  it("suma ráfagas de un usuario y emite un evento al cerrar la ventana", () => {
    n.like(like("l1", 3, 100), 0);
    n.like(like("l2", 5, 108), 400);
    n.like(like("l3", 2, 110), 900);
    expect(n.flush(999)).toEqual([]);
    const out = n.flush(1_000);
    expect(out).toHaveLength(1);
    expect(out[0]).toMatchObject({ type: "like", like: { count: 10, total: 110 } });
    expect(out[0]?.user.uniqueId).toBe("ana");
  });

  it("agrega por usuario por separado", () => {
    n.like(like("a", 1, 1), 0);
    n.like(like("b", 4, 5, bob), 0);
    const out = n.flush(1_000);
    expect(out.map((e) => [e.user.uniqueId, e.like?.count]).sort()).toEqual([
      ["ana", 1],
      ["bob", 4],
    ]);
  });

  it("no emite likes duplicados", () => {
    n.like(like("dup", 5, 5), 0);
    n.like(like("dup", 5, 5), 1);
    expect(n.flush(1_000)[0]?.like?.count).toBe(5);
  });

  it("tras emitir, una nueva ráfaga abre otra ventana", () => {
    n.like(like("a", 1, 1), 0);
    n.flush(1_000);
    n.like(like("b", 2, 3), 1_500);
    expect(n.flush(2_500)[0]?.like?.count).toBe(2);
  });
});

describe("otros eventos", () => {
  it("follow y share", () => {
    expect(n.social("follow", { common: { msgId: "f" }, user: ana }, 5)[0]?.type).toBe("follow");
    expect(n.social("share", { common: { msgId: "s" }, user: ana }, 5)[0]?.type).toBe("share");
  });

  it("join solo para la acción JOINED", () => {
    expect(n.member({ common: { msgId: "j1" }, user: ana, action: 1 }, 0)[0]?.type).toBe("join");
    expect(n.member({ common: { msgId: "j2" }, user: ana, action: 3 }, 0)).toEqual([]);
  });

  it("suscripción marca al usuario como suscriptor", () => {
    const out = n.subNotify({ common: { msgId: "s" }, user: ana }, 0);
    expect(out[0]).toMatchObject({ type: "subscribe", user: { isSubscriber: true } });
  });

  it("emote", () => {
    const out = n.emote({ common: { msgId: "e" }, user: ana, emoteList: [{ emoteId: "7", image: { urlList: ["u"] } }] }, 0);
    expect(out[0]?.chat?.emotes).toEqual([{ id: "7", image: "u" }]);
  });

  it("liveEnd vacía lo pendiente antes de cerrar", () => {
    n.like(like("a", 2, 2), 0);
    n.gift(gift({ repeatCount: 3 }), 0);
    const out = n.liveEnd(50);
    expect(out.map((e) => e.type)).toEqual(["like", "gift", "liveEnd"]);
  });

  it("chat con emotes y roles desde userIdentity", () => {
    const out = n.chat(
      {
        common: { msgId: "c" },
        user: { ...ana, payGrade: { level: 12 }, fansClub: { data: { level: 3 } } },
        userIdentity: { isModeratorOfAnchor: true, isSubscriberOfAnchor: true, isFollowerOfAnchor: false },
        content: "hola",
        emotes: [{ emote: { emoteId: "9", image: { urlList: ["img"] } } }],
      },
      0,
    );
    expect(out[0]?.user).toMatchObject({ isModerator: true, isSubscriber: true, isFollower: false, gifterLevel: 12, teamLevel: 3 });
    expect(out[0]?.chat?.emotes).toEqual([{ id: "9", image: "img" }]);
  });

  it("createTime en segundos se convierte a ms; si falta, usa now", () => {
    const sec = n.chat({ common: { msgId: "a", createTime: "1700000000" }, user: ana, content: "x" }, 5);
    const ms = n.chat({ common: { msgId: "b", createTime: "1700000000123" }, user: ana, content: "x" }, 5);
    const none = n.chat({ common: { msgId: "c" }, user: ana, content: "x" }, 5);
    expect(sec[0]?.ts).toBe(1_700_000_000_000);
    expect(ms[0]?.ts).toBe(1_700_000_000_123);
    expect(none[0]?.ts).toBe(5);
  });
});

describe("robustez (mensajes desconocidos o malformados)", () => {
  it("un mensaje sin usuario se descarta con log, sin lanzar", () => {
    expect(n.chat({ common: { msgId: "x" }, content: "hola" }, 0)).toEqual([]);
    expect(logs.some((l) => l.startsWith("warn") && l.includes("chat"))).toBe(true);
  });

  it("datos basura no lanzan y quedan en el log", () => {
    const evil = { common: { msgId: "z" }, user: ana, get repeatCount(): number { throw new Error("boom"); } } as unknown as RawGiftMessage;
    expect(() => n.gift(evil, 0)).not.toThrow();
    expect(n.gift(evil, 0)).toEqual([]);
    expect(logs.some((l) => l.startsWith("error") && l.includes("boom"))).toBe(true);
  });

  it("unknown() registra el mensaje", () => {
    n.unknown("WebcastFooMessage", { a: 1 });
    expect(logs[0]).toContain("WebcastFooMessage");
  });
});
