import { describe, expect, it } from "vitest";
import { notesFor, section, tidy } from "./changelog-notes.mjs";

const CHANGELOG = `# Changelog

## [Sin publicar]

### Añadido
- **Jarrón**: overlay nuevo que se llena
  con una meta de monedas.

## [0.2.4] - 2026-10-14

### Añadido
- **Salida de audio**: elige dónde suenan los sonidos.

## [0.2.3] - 2026-10-14

### Corregido
- Base de datos: sin \`database is locked\`.
`;

describe("changelog-notes", () => {
  it("extrae solo la sección pedida", () => {
    const body = section(CHANGELOG, "0.2.4") ?? "";
    expect(body).toContain("Salida de audio");
    expect(body).not.toContain("Jarrón");
    expect(body).not.toContain("database");
    expect(section(CHANGELOG, "9.9.9")).toBeNull();
  });

  it("acepta finales de línea de Windows", () => {
    expect(section(CHANGELOG.replace(/\n/g, "\r\n"), "0.2.3")).toContain("database is locked");
  });

  it("quita negritas y código, y une los puntos partidos en varias líneas", () => {
    const text = tidy(section(CHANGELOG, "Sin publicar") ?? "");
    expect(text).toBe("### Añadido\n- Jarrón: overlay nuevo que se llena con una meta de monedas.");
  });

  it("omite la sección «Interno»", () => {
    const text = notesFor("## [1.0.0]\n\n### Añadido\n- Algo\n\n### Interno\n- Refactor\n\n### Corregido\n- Un fallo\n", "1.0.0");
    expect(text).toBe("### Añadido\n- Algo\n\n### Corregido\n- Un fallo");
  });

  it("usa «Sin publicar» si la versión aún no tiene sección", () => {
    expect(notesFor(CHANGELOG, "0.3.0")).toContain("Jarrón");
    expect(notesFor(CHANGELOG, "0.2.3")).toContain("database is locked");
  });

  it("devuelve vacío si no hay nada que contar", () => {
    expect(notesFor("# Changelog\n\n## [Sin publicar]\n\n## [0.1.0]\n", "0.2.0")).toBe("");
    expect(notesFor("", "0.2.0")).toBe("");
  });
});
