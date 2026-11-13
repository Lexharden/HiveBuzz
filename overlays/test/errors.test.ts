import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import en from "../../src/i18n/en.json";
import es from "../../src/i18n/es.json";
import { describeError, isAppError, type AppError, type Translator } from "../../src/lib/errors";

type Tree = { [k: string]: string | Tree };
const lookup = (tree: Tree, key: string): string | undefined => {
  let cur: string | Tree | undefined = tree;
  for (const part of key.split(".")) {
    if (typeof cur !== "object") return undefined;
    cur = cur[part];
  }
  return typeof cur === "string" ? cur : undefined;
};

/** Traductor mínimo sobre un JSON de idioma (sustituye `{{param}}`). */
const translator = (tree: Tree): Translator => ({
  exists: (k) => lookup(tree, k) !== undefined,
  t: (k, p = {}) => (lookup(tree, k) ?? k).replace(/\{\{(\w+)\}\}/g, (_, name: string) => p[name] ?? ""),
});

describe("errores de los comandos", () => {
  it("reconoce el error serializado y no otras cosas", () => {
    expect(isAppError({ kind: "io", message: "E/S: x" })).toBe(true);
    expect(isAppError("texto")).toBe(false);
    expect(isAppError(null)).toBe(false);
    expect(isAppError({ message: "sin tipo" })).toBe(false);
  });

  it("traduce por código con sus parámetros", () => {
    const err: AppError = { kind: "invalid", message: "el archivo debe ser .zip", code: "file.badExtension", params: { ext: ".zip" } };
    expect(describeError(err, translator(en as Tree))).toBe("The file must be .zip.");
    expect(describeError(err, translator(es as Tree))).toBe("El archivo debe ser .zip.");
  });

  it("sin código traduce el tipo y conserva el detalle", () => {
    const err: AppError = { kind: "db", message: "base de datos: locked", detail: "locked" };
    expect(describeError(err, translator(en as Tree))).toBe("Database: locked");
  });

  it("sin traducción usa el texto original", () => {
    expect(describeError({ kind: "invalid", message: "el sonido no existe" }, translator(en as Tree))).toBe("el sonido no existe");
    expect(describeError({ kind: "invalid", message: "algo", code: "no.existe" }, translator(en as Tree))).toBe("algo");
  });
});

describe("códigos de error del backend", () => {
  const rustFiles = (dir: string): string[] =>
    readdirSync(dir).flatMap((name) => {
      const path = join(dir, name);
      return statSync(path).isDirectory() ? rustFiles(path) : name.endsWith(".rs") ? [path] : [];
    });
  const codes = new Set(
    rustFiles(join(__dirname, "../../src-tauri/src"))
      .filter((f) => !/[\\/]tests?(\.rs|[\\/])/.test(f))
      .flatMap((f) => [...readFileSync(f, "utf8").matchAll(/AppError::coded\(\s*"([\w.]+)"/g)].map((m) => m[1])),
  );

  it("se encuentran en el código", () => {
    expect(codes.size).toBeGreaterThan(5);
  });

  it.each([["es", es], ["en", en]] as const)("todos tienen texto en %s", (_lang, tree) => {
    const missing = [...codes].filter((c) => lookup(tree as Tree, `errors.code.${c}`) === undefined);
    expect(missing).toEqual([]);
  });
});
