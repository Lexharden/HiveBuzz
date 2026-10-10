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

/** Extensiones de lo que se reparte (instaladores, el paquete de actualización de macOS y sus firmas). */
const EXTENSIONS = [".exe", ".msi", ".dmg", ".appimage", ".deb", ".rpm", ".app.tar.gz", ".sig"];

/** Carpetas de `bundle/` con instaladores. De `macos/` solo interesa la actualización (`.app.tar.gz`), no el `.app`. */
const BUNDLE_DIRS = new Set(["nsis", "msi", "dmg", "appimage", "deb", "rpm", "macos"]);

export function isArtifact(file) {
  return EXTENSIONS.some((e) => file.toLowerCase().endsWith(e));
}

/**
 * ¿Es de esta versión? `target/` no se limpia entre compilaciones, así que puede haber instaladores viejos.
 * Los nombres sin versión (p. ej. `HiveBuzz.app.tar.gz`) se regeneran siempre y se aceptan.
 */
export function isForVersion(file, version) {
  const name = basename(file);
  if (!/\d+\.\d+\.\d+/.test(name)) return true;
  // Versión exacta, no como parte de otra (0.1.0 no debe aceptar 10.1.0).
  const escaped = version.replace(/[.+-]/g, "\\$&");
  return new RegExp(`(^|[^\\d.])${escaped}($|[^\\d])`).test(name);
}

/** Recorre `dir` y devuelve las rutas de todos los instaladores que contiene. */
export function findArtifacts(bundleRoot) {
  const out = [];
  if (!existsSync(bundleRoot)) return out;
  for (const sub of readdirSync(bundleRoot)) {
    if (!BUNDLE_DIRS.has(sub.toLowerCase())) continue;
    const dir = join(bundleRoot, sub);
    if (!statSync(dir).isDirectory()) continue;
    const macos = sub.toLowerCase() === "macos";
    for (const f of readdirSync(dir)) {
      const p = join(dir, f);
      if (!statSync(p).isFile() || !isArtifact(f)) continue;
      if (macos && !/\.app\.tar\.gz(\.sig)?$/i.test(f)) continue;
      out.push(p);
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
  const found = bundleRoots(join(root, "src-tauri"))
    .flatMap(findArtifacts)
    .filter((f) => isForVersion(f, version));
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
