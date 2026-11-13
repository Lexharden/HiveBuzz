import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Los nombres de `invoke("…")` del frontend no los comprueba el compilador: una errata solo se descubre con la
 * app abierta. Aquí se cruzan con los comandos registrados en `generate_handler!` (src-tauri/src/lib.rs).
 */
const root = join(__dirname, "../..");

const files = (dir: string, ext: RegExp): string[] =>
  readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? files(path, ext) : ext.test(name) ? [path] : [];
  });

const lib = readFileSync(join(root, "src-tauri/src/lib.rs"), "utf8");
const handler = lib.slice(lib.indexOf("generate_handler!["), lib.indexOf("])", lib.indexOf("generate_handler![")));
const registered = new Set([...handler.matchAll(/(?:\w+::)*(\w+),/g)].map((m) => m[1]));

const invoked = new Map<string, string>();
for (const file of files(join(root, "src"), /\.tsx?$/)) {
  for (const m of readFileSync(file, "utf8").matchAll(/invoke(?:<[^>]*>)?\(\s*"([\w:|-]+)"/g)) {
    if (m[1]) invoked.set(m[1], file.slice(root.length + 1));
  }
}

describe("comandos de Tauri", () => {
  it("se leen de lib.rs y del frontend", () => {
    expect(registered.size).toBeGreaterThan(100);
    expect(invoked.size).toBeGreaterThan(100);
  });

  it("todo lo que invoca el frontend está registrado en el backend", () => {
    const missing = [...invoked].filter(([name]) => !registered.has(name)).map(([name, file]) => `${name} (${file})`);
    expect(missing).toEqual([]);
  });
});
