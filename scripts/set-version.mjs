// Cambia la versión en TODOS los sitios a la vez (package.json, tauri.conf.json, Cargo.toml).
//
//   bun scripts/set-version.mjs 0.2.0
//   bun scripts/set-version.mjs --show     (muestra las tres y avisa si no coinciden)
//
// Después: git commit, `git tag v0.2.0` y `git push --tags` (dispara el release).

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const FILES = {
  package: join(root, "package.json"),
  tauri: join(root, "src-tauri", "tauri.conf.json"),
  cargo: join(root, "src-tauri", "Cargo.toml"),
};

export const isSemver = (v) => /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(v);

/** Versión de `[package]` en un Cargo.toml (la primera línea `version = "…"`). */
export function cargoVersion(text) {
  return /^version\s*=\s*"([^"]+)"/m.exec(text)?.[1] ?? null;
}

export function withCargoVersion(text, v) {
  return text.replace(/^version\s*=\s*"[^"]+"/m, `version = "${v}"`);
}

export function withJsonVersion(text, v) {
  return text.replace(/("version"\s*:\s*")[^"]+(")/, `$1${v}$2`);
}

export function jsonVersion(text) {
  return /"version"\s*:\s*"([^"]+)"/.exec(text)?.[1] ?? null;
}

function read() {
  return {
    package: jsonVersion(readFileSync(FILES.package, "utf8")),
    tauri: jsonVersion(readFileSync(FILES.tauri, "utf8")),
    cargo: cargoVersion(readFileSync(FILES.cargo, "utf8")),
  };
}

function main() {
  const arg = process.argv[2];
  if (!arg || arg === "--show") {
    const v = read();
    console.log(v);
    const same = new Set(Object.values(v)).size === 1;
    console.log(same ? "✅ Las tres versiones coinciden." : "❌ Las versiones NO coinciden: usa `bun scripts/set-version.mjs X.Y.Z`.");
    process.exit(same ? 0 : 1);
  }
  const version = arg.replace(/^v/, "");
  if (!isSemver(version)) {
    console.error(`❌ «${arg}» no es una versión válida (ej.: 0.2.0).`);
    process.exit(2);
  }
  writeFileSync(FILES.package, withJsonVersion(readFileSync(FILES.package, "utf8"), version));
  writeFileSync(FILES.tauri, withJsonVersion(readFileSync(FILES.tauri, "utf8"), version));
  writeFileSync(FILES.cargo, withCargoVersion(readFileSync(FILES.cargo, "utf8"), version));
  console.log(`✅ Versión ${version} en package.json, tauri.conf.json y Cargo.toml.`);
  console.log("   Recuerda: Cargo.lock se actualiza al compilar, y añade la entrada en CHANGELOG.md.");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
