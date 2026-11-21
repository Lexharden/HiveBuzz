import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { buildCatalog, indexImages, parseReference, resolveImage, slugify, sourceDir } from "./sync-gifts.mjs";

describe("sync-gifts", () => {
  it("lee la lista de HiveShock con claves en español o inglés y salta lo incompleto", () => {
    const list = parseReference(
      JSON.stringify([
        { id: 9427, nombre_en: "Pegasus", diamantes: 42999, icon_url: "https://cdn/p.webp", imagen_local: "gift_images\\9427_Pegasus.webp" },
        { id: "5655", name: " Rose ", diamonds: "1" },
        { id: 9427, nombre_en: "Repetido" },
        { nombre_en: "Sin id" },
        { id: "abc", name: "Id raro" },
        null,
      ]),
    );
    expect(list).toEqual([
      { id: 9427, name: "Pegasus", diamonds: 42999, iconUrl: "https://cdn/p.webp", imageHint: "gift_images\\9427_Pegasus.webp" },
      { id: 5655, name: "Rose", diamonds: 1, iconUrl: "", imageHint: "" },
    ]);
    expect(parseReference(JSON.stringify({ gifts: [{ id: 1, name: "A" }] }))).toHaveLength(1);
    expect(parseReference(JSON.stringify({ regalos: [{ id: 1, name: "A" }] }))).toHaveLength(1);
    expect(() => parseReference("{}")).toThrow();
  });

  it("encuentra la imagen por la ruta indicada, por id o por nombre", () => {
    const index = indexImages(["9427_Pegasus.webp", "5655.png", "finger-heart.webp", "5655_Rose.webp", "notas.txt"]);
    expect(resolveImage({ id: 1, name: "", imageHint: "otra_carpeta\\9427_Pegasus.webp" }, index)).toBe("9427_Pegasus.webp");
    expect(resolveImage({ id: 5655, name: "Rose", imageHint: "" }, index)).toBe("5655_Rose.webp"); // webp antes que png
    expect(resolveImage({ id: 77, name: "Finger Heart", imageHint: "" }, index)).toBe("finger-heart.webp");
    expect(resolveImage({ id: 78, name: "Nada", imageHint: "" }, index)).toBeNull();
    expect(slugify("  It’s Corn! ")).toBe("it-s-corn");
  });

  it("ordena por diamantes y separa lo que falta y lo que sobra", () => {
    const entries = parseReference(
      JSON.stringify([
        { id: 2, name: "Caro", diamonds: 100 },
        { id: 1, name: "Barato", diamonds: 1 },
        { id: 3, name: "Sin imagen", diamonds: 5 },
      ]),
    );
    const r = buildCatalog(entries, ["1_Barato.webp", "2_Caro.webp", "99_Viejo.webp", "leeme.txt"]);
    expect(r.gifts.map((g) => [g.id, g.image])).toEqual([
      [1, "1_Barato.webp"],
      [3, ""],
      [2, "2_Caro.webp"],
    ]);
    expect(r.used).toEqual(["1_Barato.webp", "2_Caro.webp"]);
    expect(r.missing.map((g) => g.id)).toEqual([3]);
    expect(r.unused).toEqual(["99_Viejo.webp"]);
  });

  it("toma la carpeta del argumento, del entorno o de .env, y acepta la raíz del repositorio", () => {
    expect(sourceDir([], {}, {})).toBeNull();
    expect(sourceDir(["/no/existe"], {}, {})).toBe(join("/no/existe", "config"));
    expect(sourceDir([], { HIVESHOCK_DIR: "/a" }, { HIVESHOCK_DIR: "/b" })).toBe(join("/a", "config"));
    expect(sourceDir([], {}, { HIVESHOCK_DIR: "/b" })).toBe(join("/b", "config"));
  });
});
