// Empaqueta el sidecar como binario único (Bun --compile) con el nombre que Tauri
// espera para un externalBin: binaries/tiktok-sidecar-<target-triple>[.exe]
//
//   bun run build              -> compila para el host
//   bun run build -- --release -> con minificación

import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = resolve(root, "..", "src-tauri", "binaries");

function hostTriple() {
  const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const m = /^host:\s*(\S+)/m.exec(out);
  if (!m?.[1]) throw new Error("no se pudo obtener el target triple desde `rustc -vV`");
  return m[1];
}

// target triple de Rust -> target de Bun
const BUN_TARGETS = {
  "x86_64-pc-windows-msvc": "bun-windows-x64",
  "x86_64-unknown-linux-gnu": "bun-linux-x64",
  "aarch64-unknown-linux-gnu": "bun-linux-arm64",
  "x86_64-apple-darwin": "bun-darwin-x64",
  "aarch64-apple-darwin": "bun-darwin-arm64",
};

const triple = process.env.TAURI_ENV_TARGET_TRIPLE ?? hostTriple();
const bunTarget = BUN_TARGETS[triple];
if (!bunTarget) throw new Error(`target no soportado: ${triple}`);

const ext = triple.includes("windows") ? ".exe" : "";
mkdirSync(outDir, { recursive: true });
const outfile = join(outDir, `tiktok-sidecar-${triple}${ext}`);

const args = ["build", "--compile", `--target=${bunTarget}`, join(root, "src", "index.ts"), "--outfile", outfile];
if (process.argv.includes("--release")) args.push("--minify");

console.log(`> bun ${args.join(" ")}`);
execFileSync("bun", args, { stdio: "inherit", cwd: root });
console.log(`sidecar listo: ${outfile}`);
