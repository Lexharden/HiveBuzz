// Reúne los instaladores que dejó `tauri build` en una carpeta limpia (`release/<versión>/`) y calcula sus SHA-256.
//
//   bun scripts/collect-artifacts.mjs
//
// Lo usan `scripts/build.sh` y `scripts/build.ps1`; no hace falta ejecutarlo a mano.

import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, relative } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Extensiones de lo que se reparte (instaladores y sus firmas del auto-update). */
const EXTENSIONS = [".exe", ".msi", ".dmg", ".appimage", ".deb", ".rpm", ".sig"];

/** Carpetas de `bundle/` con instaladores; el resto (p. ej. `macos/*.app`) son restos intermedios. */
const BUNDLE_DIRS = new Set(["nsis", "msi", "dmg", "appimage", "deb", "rpm"]);

export function isArtifact(file) {
  return EXTENSIONS.some((e) => file.toLowerCase().endsWith(e));
}

/** Recorre `dir` y devuelve las rutas de todos los instaladores que contiene. */
export function findArtifacts(bundleRoot) {
  const out = [];
  if (!existsSync(bundleRoot)) return out;
  for (const sub of readdirSync(bundleRoot)) {
    if (!BUNDLE_DIRS.has(sub.toLowerCase())) continue;
    const dir = join(bundleRoot, sub);
    if (!statSync(dir).isDirectory()) continue;
    for (const f of readdirSync(dir)) {
      const p = join(dir, f);
      if (statSync(p).isFile() && isArtifact(f)) out.push(p);
    }
  }
  return out;
}

/** Todas las carpetas `bundle` posibles (compilación normal y con `--target`). */
export function bundleRoots(tauriDir) {
  const target = join(tauriDir, "target");
  const roots = [join(target, "release", "bundle")];
  if (existsSync(target)) {
    for (const d of readdirSync(target)) {
      const p = join(target, d, "release", "bundle");
      if (existsSync(p)) roots.push(p);
    }
  }
  return roots;
}

export const sha256 = (file) => createHash("sha256").update(readFileSync(file)).digest("hex");

const human = (n) => (n > 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);

function main() {
  const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
  const found = bundleRoots(join(root, "src-tauri")).flatMap(findArtifacts);
  if (found.length === 0) {
    console.error("❌ No se encontró ningún instalador en src-tauri/target/**/bundle. ¿Terminó bien la compilación?");
    process.exit(1);
  }
  const outDir = join(root, "release", version);
  mkdirSync(outDir, { recursive: true });
  const lines = [];
  console.log(`\n📦 Instaladores de HiveBuzz ${version}:`);
  for (const src of found) {
    const dst = join(outDir, basename(src));
    copyFileSync(src, dst);
    const hash = sha256(dst);
    lines.push(`${hash}  ${basename(dst)}`);
    console.log(`   • ${relative(root, dst)}  (${human(statSync(dst).size)})`);
  }
  writeFileSync(join(outDir, "SHA256SUMS.txt"), lines.join("\n") + "\n");
  console.log(`   • ${relative(root, join(outDir, "SHA256SUMS.txt"))}  (comprobación de integridad)`);
  console.log("\n✅ Listo. Sube estos archivos a un release de GitHub o repártelos tal cual.");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
