// Prueba de contrato con `tiktok-live-connector`.
//
// Los demás tests del normalizador usan mensajes escritos a mano con los tipos de `raw.ts`. Estos pasan los mensajes
// por el codec protobuf REAL de la librería (codificar y decodificar), igual que llegan desde TikTok: ahí es donde
// el protobuf rellena los campos ausentes con "0" / "" / 0 y donde se notaría que la librería cambia un nombre, un
// tipo o un valor por defecto. Además, al pasar los mensajes decodificados al normalizador, `tsc` comprueba que los
// tipos reales de la librería siguen siendo asignables a los `Raw*` que él espera.
//
// Los mensajes se construyen a partir de un mensaje en blanco (decodificar 0 bytes), porque el protobuf de la librería
// exige todos los campos. No son capturas de un directo real (no se puede grabar sin una cuenta en vivo); si algún día
// se capturan, basta con guardarlos como `Uint8Array` y pasarlos por `Codec.decode`.

import {
  CommonMessageData,
  FollowInfo,
  Gift,
  ImageModel,
  User,
  UserHonor,
  WebcastChatMessage,
  WebcastEmoteChatMessage,
  WebcastGiftMessage,
  WebcastLikeMessage,
  WebcastMemberMessage,
  WebcastSocialMessage,
  WebcastSubNotifyMessage,
  EmoteModel,
} from "tiktok-live-connector";
import { beforeEach, describe, expect, it } from "vitest";
import { Normalizer } from "../src/normalizer";

interface Codec<T> {
  encode(message: T): { finish(): Uint8Array };
  decode(input: Uint8Array): T;
}

/** Mensaje con todos los campos en su valor por defecto de protobuf. */
const blank = <T>(codec: Codec<T>): T => codec.decode(new Uint8Array());
/** Ida y vuelta por el codec: lo que de verdad recibiría el sidecar. */
const wire = <T>(codec: Codec<T>, message: T): T => codec.decode(codec.encode(message).finish());
/** Igual, pero con un campo desconocido añadido al final (TikTok añade campos sin avisar). */
const withUnknownField = <T>(codec: Codec<T>, message: T): T => {
  const bytes = codec.encode(message).finish();
  const varint = (n: number): number[] => {
    const out: number[] = [];
    let v = n;
    while (v > 0x7f) {
      out.push((v & 0x7f) | 0x80);
      v = Math.floor(v / 128);
    }
    out.push(v);
    return out;
  };
  // Campo 9999, tipo varint, valor 42.
  return codec.decode(Uint8Array.from([...bytes, ...varint(9999 * 8), 42]));
};

const NOW = 1_800_000_000_000;
// `msgId` es un uint64 en el protocolo: el codec rechaza cualquier id que no sea numérico.
const img = (url: string): ImageModel => ({ ...blank(ImageModel), urlList: [url] });
const common = (msgId: string, createTime = "1730000000000"): CommonMessageData => ({ ...blank(CommonMessageData), msgId, createTime });
const user = (over: Partial<User> = {}): User => ({
  ...blank(User),
  id: "7234567890123456789",
  nickname: "Ana",
  displayId: "ana",
  avatarThumb: img("https://p16.example.com/ana.webp"),
  ...over,
});
const honor = (level: number): UserHonor => ({ ...blank(UserHonor), level });

const logs: string[] = [];
let n: Normalizer;
beforeEach(() => {
  logs.length = 0;
  n = new Normalizer({ log: (l, m) => logs.push(`${l}: ${m}`), likeWindowMs: 1_000 });
});

describe("contrato con tiktok-live-connector · regalos", () => {
  const gift = (over: Partial<WebcastGiftMessage> & { name?: string; type?: number; coins?: number } = {}): WebcastGiftMessage => {
    const { name = "Rose", type = 1, coins = 1, ...rest } = over;
    return {
      ...blank(WebcastGiftMessage),
      common: common("7300000000000000001"),
      user: user(),
      giftId: "5655",
      groupId: "7400000000000000001",
      repeatCount: 1,
      repeatEnd: 0,
      gift: { ...blank(Gift), id: "5655", name, type, diamondCount: coins, image: img("https://p16.example.com/rose.webp") },
      ...rest,
    };
  };

  it("un regalo combinable espera a repeatEnd y sale con el conteo final y las monedas totales", () => {
    expect(n.gift(wire(WebcastGiftMessage, gift({ repeatCount: 3, repeatEnd: 0 })), NOW)).toEqual([]);
    const [ev] = n.gift(wire(WebcastGiftMessage, gift({ common: common("7300000000000000002"), repeatCount: 5, repeatEnd: 1 })), NOW);
    expect(ev).toMatchObject({
      type: "gift",
      id: "7300000000000000002",
      user: { id: "7234567890123456789", uniqueId: "ana", nickname: "Ana", avatar: "https://p16.example.com/ana.webp" },
      gift: { id: 5655, name: "Rose", count: 5, coins: 5, streakable: true, image: "https://p16.example.com/rose.webp" },
      ts: 1730000000000,
    });
  });

  it("un regalo no combinable sale al instante, sin esperar a repeatEnd", () => {
    const [ev] = n.gift(wire(WebcastGiftMessage, gift({ name: "Galaxy", type: 2, coins: 1000, repeatEnd: 0 })), NOW);
    expect(ev).toMatchObject({ type: "gift", gift: { name: "Galaxy", coins: 1000, count: 1, streakable: false } });
  });

  it("con un campo desconocido añadido por TikTok al final, sigue funcionando igual", () => {
    const [ev] = n.gift(withUnknownField(WebcastGiftMessage, gift({ name: "Galaxy", type: 2, coins: 1000 })), NOW);
    expect(ev?.gift?.name).toBe("Galaxy");
  });

  it("el nivel de donador sale de payGrade", () => {
    const msg = wire(WebcastGiftMessage, gift({ name: "Galaxy", type: 2, user: user({ payGrade: honor(12) }) }));
    expect(n.gift(msg, NOW)[0]?.user.gifterLevel).toBe(12);
  });

  it("un id que el protobuf dejó en \"0\" no cuenta como id", () => {
    const [ev] = n.gift(wire(WebcastGiftMessage, gift({ name: "Galaxy", type: 2, user: user({ id: "0" }) })), NOW);
    expect(ev?.user.id).toBe("ana"); // sin id numérico se usa el @usuario
  });
});

describe("contrato con tiktok-live-connector · chat, sociales y entradas", () => {
  it("chat: texto y usuario", () => {
    const msg = wire(WebcastChatMessage, { ...blank(WebcastChatMessage), common: common("7300000000000000003"), user: user(), content: "¡hola a todos! 🔥" });
    expect(n.chat(msg, NOW)[0]).toMatchObject({ type: "chat", id: "7300000000000000003", chat: { text: "¡hola a todos! 🔥" }, user: { uniqueId: "ana" } });
  });

  it("chat: el moderador y el seguidor se reconocen por los campos reales del protocolo", () => {
    const base = user({ followInfo: { ...blank(FollowInfo), followStatus: "1" } });
    const msg = wire(WebcastChatMessage, { ...blank(WebcastChatMessage), common: common("7300000000000000004"), user: base, content: "x" });
    expect(n.chat(msg, NOW)[0]?.user.isFollower).toBe(true);
  });

  it("follow y share", () => {
    const social = (id: string) => wire(WebcastSocialMessage, { ...blank(WebcastSocialMessage), common: common(id), user: user() });
    expect(n.social("follow", social("7300000000000000005"), NOW)[0]).toMatchObject({ type: "follow", user: { uniqueId: "ana", isFollower: true } });
    expect(n.social("share", social("7300000000000000006"), NOW)[0]).toMatchObject({ type: "share" });
  });

  it("entrada al LIVE (solo la acción JOINED) y suscripción", () => {
    const member = (id: string, action: number) => wire(WebcastMemberMessage, { ...blank(WebcastMemberMessage), common: common(id), user: user(), action });
    expect(n.member(member("7300000000000000007", 1), NOW)[0]).toMatchObject({ type: "join" });
    expect(n.member(member("7300000000000000008", 0), NOW)).toEqual([]);
    const sub = wire(WebcastSubNotifyMessage, { ...blank(WebcastSubNotifyMessage), common: common("7300000000000000009"), user: user() });
    expect(n.subNotify(sub, NOW)[0]).toMatchObject({ type: "subscribe", user: { isSubscriber: true } });
  });

  it("emote: el id y la imagen del emote", () => {
    const emote: EmoteModel = { ...blank(EmoteModel), emoteId: "emote-77", image: img("https://p16.example.com/e.webp") };
    const msg = wire(WebcastEmoteChatMessage, { ...blank(WebcastEmoteChatMessage), common: common("7300000000000000010"), user: user(), emoteList: [emote] });
    const [ev] = n.emote(msg, NOW);
    expect(ev).toMatchObject({ type: "emote", chat: { emotes: [{ id: "emote-77", image: "https://p16.example.com/e.webp" }] } });
  });

  it("likes: se agregan por usuario y salen al cerrar la ventana", () => {
    const like = (id: string, count: number, total: string) => wire(WebcastLikeMessage, { ...blank(WebcastLikeMessage), common: common(id), user: user(), count, total });
    expect(n.like(like("7300000000000000011", 3, "100"), NOW)).toEqual([]);
    expect(n.like(like("7300000000000000012", 2, "105"), NOW + 100)).toEqual([]);
    const [ev] = n.flush(NOW + 2_000);
    expect(ev).toMatchObject({ type: "like", like: { count: 5, total: 105 } });
  });
});

describe("contrato con tiktok-live-connector · mensajes vacíos o raros", () => {
  it("con todos los campos en su valor por defecto ningún manejador lanza ni inventa eventos", () => {
    const empty = {
      gift: wire(WebcastGiftMessage, blank(WebcastGiftMessage)),
      chat: wire(WebcastChatMessage, blank(WebcastChatMessage)),
      like: wire(WebcastLikeMessage, blank(WebcastLikeMessage)),
      social: wire(WebcastSocialMessage, blank(WebcastSocialMessage)),
      member: wire(WebcastMemberMessage, blank(WebcastMemberMessage)),
      sub: wire(WebcastSubNotifyMessage, blank(WebcastSubNotifyMessage)),
      emote: wire(WebcastEmoteChatMessage, blank(WebcastEmoteChatMessage)),
    };
    expect(n.gift(empty.gift, NOW)).toEqual([]);
    expect(n.chat(empty.chat, NOW)).toEqual([]);
    expect(n.like(empty.like, NOW)).toEqual([]);
    expect(n.social("follow", empty.social, NOW)).toEqual([]);
    expect(n.social("share", empty.social, NOW)).toEqual([]);
    expect(n.member({ ...empty.member, action: 1 }, NOW)).toEqual([]);
    expect(n.subNotify(empty.sub, NOW)).toEqual([]);
    expect(n.emote(empty.emote, NOW)).toEqual([]);
    expect(n.flushAll(NOW + 60_000)).toEqual([]);
    expect(logs.some((l) => l.startsWith("error"))).toBe(false);
  });

  it("un usuario sin id ni @ (todo por defecto) se descarta con un aviso, no se cuela como usuario vacío", () => {
    const nobody = user({ id: "0", displayId: "", nickname: "" });
    const msg = wire(WebcastChatMessage, { ...blank(WebcastChatMessage), common: common("7300000000000000013"), user: nobody, content: "hola" });
    expect(n.chat(msg, NOW)).toEqual([]);
    expect(logs.some((l) => l.includes("sin usuario"))).toBe(true);
  });

  it("sin hora en el mensaje se usa la del sistema; en segundos, se pasa a milisegundos", () => {
    const msg = (id: string, createTime: string) => wire(WebcastChatMessage, { ...blank(WebcastChatMessage), common: common(id, createTime), user: user(), content: "x" });
    expect(n.chat(msg("7300000000000000014", "0"), NOW)[0]?.ts).toBe(NOW);
    expect(n.chat(msg("7300000000000000015", "1730000000"), NOW)[0]?.ts).toBe(1730000000000);
  });

  it("el mismo mensaje reenviado (mismo msgId) no se cuenta dos veces", () => {
    const msg = wire(WebcastChatMessage, { ...blank(WebcastChatMessage), common: common("7300000000000000016"), user: user(), content: "x" });
    expect(n.chat(msg, NOW)).toHaveLength(1);
    expect(n.chat(msg, NOW + 1)).toHaveLength(0);
  });
});
