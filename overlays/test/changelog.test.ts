import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { compareVersions, MAX_RELEASES, parseChangelog, parseInline, planWhatsNew } from "../../src/lib/changelog";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const real = readFileSync(join(root, "CHANGELOG.md"), "utf8");
const packageVersion = (JSON.parse(readFileSync(join(root, "package.json"), "utf8")) as { version: string }).version;

const SAMPLE = `# Changelog

## [Sin publicar]

### Añadido
- Algo que aún no salió.

## [0.5.0] - 2026-11-04

### Añadido
- **Uno**: primer punto con \`código\`.
- Segundo punto que sigue
  en otra línea sangrada.

### Interno
- Cosa de CI que el usuario no necesita.

## [0.4.3] - 2026-10-26

### Mejorado
- Mejora vieja.

## [0.4.2] - 2026-10-25

### Corregido
- Arreglo viejo.

## [0.4.1] - 2026-10-24

### Corregido
- Otro arreglo.

## [0.4.0] - 2026-10-21

### Interno
- Solo cosas internas.
`;

describe("changelog", () => {
  const releases = parseChangelog(SAMPLE);

  it("lee las versiones publicadas, de la más nueva a la más vieja, sin «Sin publicar»", () => {
    expect(releases.map((r) => r.version)).toEqual(["0.5.0", "0.4.3", "0.4.2", "0.4.1"]);
    expect(releases[0]?.date).toBe("2026-11-04");
  });

  it("oculta la sección «Interno» y descarta versiones que solo tenían eso", () => {
    expect(releases[0]?.sections.map((s) => s.title)).toEqual(["Añadido"]);
    expect(releases.some((r) => r.version === "0.4.0")).toBe(false);
  });

  it("une las líneas sangradas al punto anterior y acepta saltos de línea de Windows", () => {
    expect(releases[0]?.sections[0]?.items[1]).toBe("Segundo punto que sigue en otra línea sangrada.");
    expect(parseChangelog(SAMPLE.replace(/\n/g, "\r\n"))).toEqual(releases);
  });

  it("compara versiones numéricamente (0.10.0 es mayor que 0.9.0)", () => {
    expect(compareVersions("0.4.3", "0.5.0")).toBeLessThan(0);
    expect(compareVersions("0.10.0", "0.9.0")).toBeGreaterThan(0);
    expect(compareVersions("1.0", "1.0.0")).toBe(0);
    expect(compareVersions("0.5.0-beta.1", "0.5.0")).toBe(0);
  });

  it("trocea negritas, código y cursivas sin generar HTML", () => {
    expect(parseInline("**Fuentes** en *Biblioteca* con `{nickname}` y <b>texto</b>")).toEqual([
      { kind: "bold", text: "Fuentes" },
      { kind: "text", text: " en " },
      { kind: "italic", text: "Biblioteca" },
      { kind: "text", text: " con " },
      { kind: "code", text: "{nickname}" },
      { kind: "text", text: " y <b>texto</b>" },
    ]);
    expect(parseInline("sin formato")).toEqual([{ kind: "text", text: "sin formato" }]);
  });
});

describe("cuándo se enseñan las novedades", () => {
  const releases = parseChangelog(SAMPLE);
  const versions = (seen: string | null, current: string, existing = false) => planWhatsNew(releases, current, seen, existing).show.map((r) => r.version);

  it("una instalación nueva no enseña nada y anota la versión", () => {
    expect(planWhatsNew(releases, "0.5.0", null)).toEqual({ show: [], markSeen: true });
  });

  it("quien venía de una versión sin esta ventana ve solo la actual", () => {
    expect(planWhatsNew(releases, "0.5.0", null, true)).toMatchObject({ markSeen: true, show: [{ version: "0.5.0" }] });
  });

  it("al actualizar enseña lo que hay entre la versión vista (sin incluirla) y la actual", () => {
    expect(versions("0.4.3", "0.5.0")).toEqual(["0.5.0"]);
    expect(versions("0.4.1", "0.5.0")).toEqual(["0.5.0", "0.4.3", "0.4.2"]);
  });

  it("no enseña más de tres versiones aunque se haya saltado muchas", () => {
    expect(versions("0.1.0", "0.5.0").length).toBe(MAX_RELEASES);
  });

  it("no enseña nada si la versión no ha cambiado o es más vieja (se vuelve atrás)", () => {
    expect(versions("0.5.0", "0.5.0")).toEqual([]);
    expect(versions("0.6.0", "0.5.0")).toEqual([]);
  });

  it("no enseña versiones más nuevas que la instalada", () => {
    expect(versions("0.4.1", "0.4.3")).toEqual(["0.4.3", "0.4.2"]);
  });
});

describe("el CHANGELOG.md real", () => {
  const releases = parseChangelog(real);

  it("la versión actual del proyecto tiene notas que la ventana puede enseñar", () => {
    const current = releases.find((r) => r.version === packageVersion);
    expect(current, `CHANGELOG.md necesita una sección [${packageVersion}] con puntos`).toBeDefined();
    expect(current?.sections.flatMap((s) => s.items).length).toBeGreaterThan(0);
  });

  it("todas las versiones se leen con fecha y puntos no vacíos", () => {
    expect(releases.length).toBeGreaterThan(5);
    for (const r of releases) {
      expect(r.date, r.version).toMatch(/^\d{4}-\d{2}-\d{2}$/);
      for (const s of r.sections) for (const item of s.items) expect(item.trim().length, `${r.version} · ${s.title}`).toBeGreaterThan(0);
    }
  });
});
