import { api } from "./api";
import type { GiftView } from "./types";

/** Minúsculas y sin acentos, para buscar «corazon» y encontrar «Corazón». */
export const fold = (s: string): string =>
  s
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .trim();

/** Todos los nombres del regalo (oficial, en español y alias). */
export const giftNames = (g: GiftView): string[] => [g.nameEn, g.nameEs, ...g.also].filter((n): n is string => !!n);

/** El nombre que manda TikTok en directo (el oficial); si no lo hay, el que se conozca. */
export const primaryName = (g: GiftView): string => g.nameEn ?? g.nameEs ?? g.also[0] ?? g.displayName;

/** Busca por nombre, alias o id, sin distinguir mayúsculas ni acentos. Respeta el orden del catálogo (por precio). */
export function filterGifts(gifts: GiftView[], query: string, limit = Infinity): GiftView[] {
  const q = fold(query);
  const out: GiftView[] = [];
  for (const g of gifts) {
    if (out.length >= limit) break;
    const hit =
      q === "" ||
      giftNames(g).some((n) => fold(n).includes(q)) ||
      [g.id, ...g.otherIds].some((id) => id !== null && String(id) === q);
    if (hit) out.push(g);
  }
  return out;
}

/** Lo que guarda una regla, meta o timer para elegir un regalo. */
export interface GiftSelection {
  giftId?: number;
  giftName?: string;
}

/** Lo escrito a mano: «#5655» o «5655» es un id; cualquier otra cosa, un nombre. */
export function parseTyped(v: string): GiftSelection {
  const s = v.trim();
  const id = /^#?(\d{1,15})$/.exec(s);
  if (id) return { giftId: Number(id[1]) };
  return s ? { giftName: v } : {};
}

/** El regalo que eligió una regla, meta o timer: por id (también variantes) y, si no, por nombre o alias. */
export function findGift(gifts: GiftView[], sel: GiftSelection): GiftView | undefined {
  if (sel.giftId !== undefined) {
    const byId = gifts.find((g) => g.id === sel.giftId || g.otherIds.includes(sel.giftId as number));
    if (byId) return byId;
  }
  const name = sel.giftName ? fold(sel.giftName) : "";
  return name ? gifts.find((g) => giftNames(g).some((n) => fold(n) === name)) : undefined;
}

/** Imagen del regalo: la local (servida por la app) o, si no hay, el icono oficial. */
export function giftImageUrl(g: GiftView, base: string | null | undefined, token: string | null | undefined): string | undefined {
  if (g.image && base && token) return `${base}/${encodeURIComponent(g.image)}?token=${encodeURIComponent(token)}`;
  return g.iconUrl || undefined;
}

/** Texto corto para resúmenes: «Rosa / Rose · 1 💎». */
export const giftSummary = (g: GiftView): string => (g.diamonds ? `${g.displayName} · ${g.diamonds} 💎` : g.displayName);

// ---- Caché compartida entre editores ----

export interface GiftCatalog {
  gifts: GiftView[];
  base: string | null;
  token: string | null;
}

let cached: Promise<GiftCatalog> | null = null;
const listeners = new Set<() => void>();

/** El catálogo entero (unos cientos de regalos), pedido una vez y compartido. */
export function loadGiftCatalog(): Promise<GiftCatalog> {
  cached ??= Promise.all([api.listGifts(), api.getAppInfo()])
    .then(([gifts, info]) => ({ gifts, base: info.giftsBase, token: info.overlayToken }))
    .catch((e: unknown) => {
      cached = null;
      throw e;
    });
  return cached;
}

/** Tras editar el catálogo: la próxima lectura vuelve a pedirlo y los selectores abiertos se refrescan. */
export function invalidateGiftCatalog() {
  cached = null;
  for (const l of listeners) l();
}

export function onGiftCatalogChange(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}
