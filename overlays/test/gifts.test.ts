import { describe, expect, it, vi } from "vitest";

// `gifts.ts` importa la API de Tauri para la caché; aquí solo se prueban las funciones puras.
vi.mock("../../src/lib/api", () => ({ api: {} }));

const { filterGifts, findGift, fold, giftImageUrl, giftSummary, parseTyped, primaryName } = await import("../../src/lib/gifts");
import type { GiftView } from "../../src/lib/types";

const gift = (p: Partial<GiftView>): GiftView => ({
  key: String(p.id ?? p.nameEn),
  id: null,
  otherIds: [],
  nameEn: null,
  nameEs: null,
  also: [],
  displayName: p.nameEs && p.nameEn ? `${p.nameEs} / ${p.nameEn}` : (p.nameEs ?? p.nameEn ?? ""),
  diamonds: null,
  seen: 0,
  lastSeen: null,
  image: "",
  iconUrl: "",
  referenceOnly: true,
  manual: false,
  ...p,
});

const rose = gift({ id: 5655, nameEn: "Rose", nameEs: "Rosa", diamonds: 1, image: "5655_Rose.webp", iconUrl: "https://cdn/rose.webp" });
const heart = gift({ id: 5586, nameEn: "Hearts", nameEs: "Corazón", also: ["Corazones"], diamonds: 199 });
const bear = gift({ id: 5566, otherIds: [5582], nameEn: "Mishka Bear", diamonds: 100 });
const all = [rose, bear, heart];

describe("catálogo de regalos (interfaz)", () => {
  it("busca sin mayúsculas ni acentos, por alias o por id", () => {
    expect(fold("  CORAZÓN ")).toBe("corazon");
    expect(filterGifts(all, "corazon")).toEqual([heart]);
    expect(filterGifts(all, "rosa")).toEqual([rose]);
    expect(filterGifts(all, "5582")).toEqual([bear]);
    expect(filterGifts(all, "")).toEqual(all);
    expect(filterGifts(all, "", 2)).toEqual([rose, bear]);
  });

  it("encuentra lo elegido por id (también variantes) o por nombre", () => {
    expect(findGift(all, { giftId: 5582 })).toBe(bear);
    expect(findGift(all, { giftName: "corazones" })).toBe(heart);
    expect(findGift(all, { giftId: 1, giftName: "Rose" })).toBe(rose);
    expect(findGift(all, {})).toBeUndefined();
  });

  it("usa la imagen local con token y, si no, el icono oficial", () => {
    expect(giftImageUrl(rose, "http://127.0.0.1:1/gifts", "t k")).toBe("http://127.0.0.1:1/gifts/5655_Rose.webp?token=t%20k");
    expect(giftImageUrl(rose, null, "t")).toBe("https://cdn/rose.webp");
    expect(giftImageUrl(heart, "http://x", "t")).toBeUndefined();
  });

  it("nombres y resúmenes", () => {
    expect(primaryName(heart)).toBe("Hearts");
    expect(primaryName(gift({ nameEs: "Solo ES" }))).toBe("Solo ES");
    expect(giftSummary(rose)).toBe("Rosa / Rose · 1 💎");
  });

  it("lo escrito a mano: un número es un id; lo demás, un nombre", () => {
    expect(parseTyped("#5655")).toEqual({ giftId: 5655 });
    expect(parseTyped(" 42 ")).toEqual({ giftId: 42 });
    expect(parseTyped("Rosa")).toEqual({ giftName: "Rosa" });
    expect(parseTyped("   ")).toEqual({});
  });
});
