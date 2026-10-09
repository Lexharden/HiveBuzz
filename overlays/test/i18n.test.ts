import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import en from "../../src/i18n/en.json";
import es from "../../src/i18n/es.json";

type Tree = { [k: string]: string | Tree };

function flatten(tree: Tree, prefix = ""): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(tree)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (typeof v === "string") out[key] = v;
    else Object.assign(out, flatten(v, key));
  }
  return out;
}

/** Marcadores `{{x}}` (i18next) y `{x}` (variables de reglas): deben coincidir entre idiomas. */
const placeholders = (s: string): string[] => [...(s.match(/\{\{?\w+\}?\}/g) ?? [])].sort();

describe("i18n de la interfaz", () => {
  const [es_, en_] = [flatten(es as Tree), flatten(en as Tree)];

  it("el inglés tiene exactamente las mismas claves que el español", () => {
    expect(Object.keys(en_).sort()).toEqual(Object.keys(es_).sort());
  });

  it("no hay textos vacíos", () => {
    for (const [k, v] of [...Object.entries(es_), ...Object.entries(en_)]) expect(v.trim(), k).not.toBe("");
  });

  it("los marcadores {{x}} y {x} coinciden en ambos idiomas", () => {
    for (const k of Object.keys(es_)) expect(placeholders(en_[k] ?? ""), k).toEqual(placeholders(es_[k] ?? ""));
  });
});

describe("i18n de los overlays (HB.t)", () => {
  const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "../common.js"), "utf8");
  const dictOf = (lang: string): string[] => {
    const start = src.indexOf(`    ${lang}: {`);
    expect(start, `diccionario ${lang}`).toBeGreaterThan(-1);
    const end = src.indexOf("\n    }", start);
    return [...src.slice(start, end).matchAll(/"([\w.]+)":/g)].map((m) => m[1] as string).sort();
  };

  it("cada idioma tiene las mismas claves", () => {
    expect(dictOf("en")).toEqual(dictOf("es"));
  });
});
