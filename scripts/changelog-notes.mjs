// Notas de una versión a partir de CHANGELOG.md: las usa el release de GitHub como cuerpo y, de ahí, la app
// (Ajustes → Actualizaciones muestra el cuerpo del release en texto plano, y latest.json lo copia como «notes»).
//
//   bun scripts/changelog-notes.mjs 0.3.0      imprime las notas de [0.3.0] (si no existe, las de [Sin publicar])
//
// Con la sección vacía o inexistente termina con código 1 y no imprime nada: el flujo decide qué hacer.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Cuerpo (sin el título) de la sección `## [titulo]`, o null si no existe. */
export function section(changelog, title) {
  const lines = changelog.replace(/\r\n/g, "\n").split("\n");
  const wanted = `## [${title}]`.toLowerCase();
  const start = lines.findIndex((l) => l.trim().toLowerCase().startsWith(wanted));
  if (start === -1) return null;
  const rest = lines.slice(start + 1);
  const end = rest.findIndex((l) => /^## /.test(l));
  return (end === -1 ? rest : rest.slice(0, end)).join("\n");
}

/**
 * Deja la sección legible tanto en GitHub como en texto plano: sin negritas ni comillas de código, y con
 * cada punto de la lista en una sola línea (las continuaciones sangradas del CHANGELOG se unen).
 */
export function tidy(body) {
  const out = [];
  let internal = false;
  for (const raw of body.split("\n")) {
    // «### Interno» (CI, refactors…) es para quien mantiene el código, no para quien usa la app.
    if (/^### /.test(raw)) internal = /^###\s+interno\b/i.test(raw);
    if (internal) continue;
    const line = raw.replace(/\*\*/g, "").replace(/`/g, "").trimEnd();
    if (/^\s+\S/.test(line) && out.length > 0 && /^- /.test(out[out.length - 1] ?? "")) {
      out[out.length - 1] += ` ${line.trim()}`;
    } else {
      out.push(line);
    }
  }
  return out
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

/** Notas de `version`; si su sección no existe o está vacía, las de «Sin publicar». Vacío si no hay nada. */
export function notesFor(changelog, version) {
  for (const title of [version, "Sin publicar"]) {
    const body = section(changelog, title);
    const text = body === null ? "" : tidy(body);
    if (text) return text;
  }
  return "";
}

function main() {
  const version = (process.argv[2] ?? "").replace(/^v/, "");
  if (!version) {
    console.error("Uso: bun scripts/changelog-notes.mjs <versión>");
    process.exit(2);
  }
  const text = notesFor(readFileSync(join(root, "CHANGELOG.md"), "utf8"), version);
  if (!text) {
    console.error(`CHANGELOG.md no tiene notas para ${version} ni en «Sin publicar».`);
    process.exit(1);
  }
  console.log(text);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
