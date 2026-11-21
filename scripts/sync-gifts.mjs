// Trae la lista oficial de regalos de TikTok y sus imágenes desde HiveShock (la fuente de verdad) a HiveBuzz.
//
//   bun scripts/sync-gifts.mjs [carpeta]   # la carpeta `config` de HiveShock (o define HIVESHOCK_DIR en .env)
//   bun run gifts:sync
//
// Lee `tiktok_gifts.json` y `gifts-images/` y deja en `src-tauri/resources/gifts/`:
//   - `gifts.json`: la lista normalizada ({ id, name, diamonds, iconUrl, image }) ordenada por diamantes;
//   - `images/`: solo las imágenes que usa la lista (las que ya no se usan se borran).
// Sin dependencias. Termina con error si la lista no se puede leer o queda vacía.

import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, extname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseEnv } from "./with-env.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
export const DEST = join(root, "src-tauri", "resources", "gifts");
const IMAGE_EXTENSIONS = [".webp", ".png", ".jpg", ".jpeg"];

function pick(obj, names) {
  for (const n of names) if (obj[n] !== undefined && obj[n] !== null) return obj[n];
  return undefined;
}

function text(v) {
  if (typeof v === "string") return v.trim();
  if (typeof v === "number" && Number.isFinite(v)) return String(v);
  return "";
}

/**
 * Lista oficial de HiveShock: un array o `{ gifts | regalos: [...] }`, claves en español o inglés, id como número o
 * texto. Las entradas sin id numérico se saltan; si un id se repite, gana el primero.
 */
export function parseReference(json) {
  let data = JSON.parse(json);
  if (data && !Array.isArray(data) && typeof data === "object") data = pick(data, ["gifts", "regalos"]);
  if (!Array.isArray(data)) throw new Error("la lista de regalos no es un array ni { gifts: [...] }");
  const seen = new Set();
  const out = [];
  for (const el of data) {
    if (!el || typeof el !== "object") continue;
    const id = text(pick(el, ["id", "gift_id", "giftId"]));
    if (!/^\d{1,18}$/.test(id) || seen.has(id)) continue;
    seen.add(id);
    const diamonds = Number(pick(el, ["diamantes", "diamonds", "diamond_count", "diamondCount"]));
    out.push({
      id: Number(id),
      name: text(pick(el, ["nombre_en", "name", "nameEn", "nombre"])),
      diamonds: Number.isInteger(diamonds) && diamonds >= 0 ? diamonds : null,
      iconUrl: text(pick(el, ["icon_url", "iconUrl", "icon"])),
      imageHint: text(pick(el, ["imagen_local", "image", "imageLocal", "imagen"])),
    });
  }
  return out;
}

/** Igual que `GiftImages.Slugify` de HiveShock: minúsculas y todo lo que no sea a-z/0-9 → «-». */
export function slugify(name) {
  return name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
}

/** Índice de la carpeta de imágenes (mismo orden de preferencia que HiveShock: webp primero, id menor primero). */
export function indexImages(files) {
  const rank = (f) => IMAGE_EXTENSIONS.indexOf(extname(f).toLowerCase());
  const idOf = (f) => /^(\d+)(?:[_\-\s]+(.*))?$/.exec(basename(f, extname(f)));
  const sorted = files
    .filter((f) => rank(f) >= 0)
    .sort((a, b) => rank(a) - rank(b) || Number(idOf(a)?.[1] ?? Infinity) - Number(idOf(b)?.[1] ?? Infinity) || a.localeCompare(b));
  const byFile = new Map();
  const byId = new Map();
  const bySlug = new Map();
  const put = (m, k, v) => k && !m.has(k) && m.set(k, v);
  for (const f of sorted) {
    put(byFile, f.toLowerCase(), f);
    const m = idOf(f);
    if (m) {
      put(byId, m[1], f);
      put(bySlug, slugify((m[2] ?? "").replace(/_/g, " ")), f);
    } else {
      put(bySlug, slugify(basename(f, extname(f)).replace(/_/g, " ")), f);
    }
  }
  return { byFile, byId, bySlug };
}

/** Imagen de un regalo: la indicada en la lista (solo cuenta el nombre de archivo), su id o su nombre. */
export function resolveImage(entry, index) {
  const hinted = entry.imageHint ? basename(entry.imageHint.replace(/\\/g, "/")).toLowerCase() : "";
  return (hinted && index.byFile.get(hinted)) || index.byId.get(String(entry.id)) || (entry.name && index.bySlug.get(slugify(entry.name))) || null;
}

/** Lista final y lo que conviene avisar. */
export function buildCatalog(entries, imageFiles) {
  const index = indexImages(imageFiles);
  const used = new Set();
  const missing = [];
  const gifts = entries.map((e) => {
    const image = resolveImage(e, index);
    if (image) used.add(image);
    else missing.push(e);
    return { id: e.id, name: e.name, diamonds: e.diamonds, iconUrl: e.iconUrl, image: image ?? "" };
  });
  gifts.sort((a, b) => (a.diamonds ?? Infinity) - (b.diamonds ?? Infinity) || a.name.localeCompare(b.name, "en") || a.id - b.id);
  const unused = imageFiles.filter((f) => IMAGE_EXTENSIONS.includes(extname(f).toLowerCase()) && !used.has(f));
  return { gifts, used: [...used].sort(), missing, unused };
}

/** Carpeta `config` de HiveShock: argumento, variable de entorno o `.env`. */
export function sourceDir(argv, env, dotenv) {
  const dir = argv[0] ?? env.HIVESHOCK_DIR ?? dotenv.HIVESHOCK_DIR ?? "";
  if (!dir) return null;
  // Se acepta la raíz del repositorio de HiveShock o directamente su carpeta `config`.
  return existsSync(join(dir, "tiktok_gifts.json")) ? dir : join(dir, "config");
}

function main() {
  const dotenvPath = join(root, ".env");
  const dotenv = existsSync(dotenvPath) ? parseEnv(readFileSync(dotenvPath, "utf8")) : {};
  const src = sourceDir(process.argv.slice(2), process.env, dotenv);
  if (!src) {
    console.error("Uso: bun scripts/sync-gifts.mjs <carpeta de HiveShock>   (o define HIVESHOCK_DIR en .env)");
    process.exit(2);
  }
  const listPath = join(src, "tiktok_gifts.json");
  const imagesDir = join(src, "gifts-images");
  if (!existsSync(listPath)) {
    console.error(`❌ No existe ${listPath}`);
    process.exit(1);
  }
  const entries = parseReference(readFileSync(listPath, "utf8"));
  if (entries.length === 0) {
    console.error("❌ La lista de regalos está vacía: no se toca nada.");
    process.exit(1);
  }
  const files = existsSync(imagesDir) ? readdirSync(imagesDir).filter((f) => statSync(join(imagesDir, f)).isFile()) : [];
  const { gifts, used, missing, unused } = buildCatalog(entries, files);

  const destImages = join(DEST, "images");
  mkdirSync(destImages, { recursive: true });
  for (const f of used) copyFileSync(join(imagesDir, f), join(destImages, f));
  const keep = new Set(used);
  let removed = 0;
  for (const f of readdirSync(destImages)) {
    if (!keep.has(f)) {
      rmSync(join(destImages, f), { force: true });
      removed++;
    }
  }
  const out = { _help: "Lista oficial de regalos de TikTok. Generada por scripts/sync-gifts.mjs desde HiveShock: no editar a mano.", gifts };
  writeFileSync(join(DEST, "gifts.json"), JSON.stringify(out, null, 1) + "\n");

  console.log(`✅ ${gifts.length} regalos, ${used.length} imágenes copiadas${removed ? `, ${removed} borradas` : ""}.`);
  if (missing.length) console.warn(`ℹ️  ${missing.length} regalos sin imagen: ${missing.slice(0, 10).map((g) => `${g.id} ${g.name}`).join(", ")}${missing.length > 10 ? "…" : ""}`);
  if (unused.length) console.warn(`ℹ️  ${unused.length} imágenes de HiveShock que no usa la lista (no se copian).`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) main();
