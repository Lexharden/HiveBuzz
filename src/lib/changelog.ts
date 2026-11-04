// Lectura de CHANGELOG.md para la ventana de «Novedades». Es lógica pura (sin React ni Tauri) para poder probarla.
// El formato es el de Keep a Changelog que ya usa el proyecto: `## [0.5.0] - 2026-11-04`, `### Añadido` y puntos `- …`
// que pueden continuar en líneas sangradas.

export interface ReleaseSection {
  title: string;
  items: string[];
}

export interface Release {
  version: string;
  /** `YYYY-MM-DD`, o `null` si el título no la trae. */
  date: string | null;
  sections: ReleaseSection[];
}

/** Secciones que son para quien mantiene el código, no para quien usa la app (igual que en `scripts/changelog-notes.mjs`). */
const HIDDEN_SECTIONS = /^interno\b/i;
/** Máximo de versiones que se muestran de golpe si el usuario se saltó varias. */
export const MAX_RELEASES = 3;

/** Versiones publicadas del changelog, de la más nueva a la más vieja. «Sin publicar» no cuenta. */
export function parseChangelog(raw: string): Release[] {
  const releases: Release[] = [];
  let release: Release | null = null;
  let section: ReleaseSection | null = null;
  let hidden = false;

  for (const line of raw.replace(/\r\n/g, "\n").split("\n")) {
    const head = /^##\s+\[([^\]]+)\](?:\s*-\s*(\d{4}-\d{2}-\d{2}))?/.exec(line);
    if (head) {
      const version = head[1] ?? "";
      release = /^\d+\.\d+\.\d+/.test(version) ? { version, date: head[2] ?? null, sections: [] } : null;
      if (release) releases.push(release);
      section = null;
      hidden = false;
      continue;
    }
    if (!release) continue;

    const sub = /^###\s+(.+?)\s*$/.exec(line);
    if (sub) {
      hidden = HIDDEN_SECTIONS.test(sub[1] ?? "");
      section = hidden ? null : { title: sub[1] ?? "", items: [] };
      if (section) release.sections.push(section);
      continue;
    }
    if (hidden || !section) continue;

    const bullet = /^-\s+(.*\S)\s*$/.exec(line);
    if (bullet) {
      section.items.push(bullet[1] ?? "");
    } else if (/^\s+\S/.test(line) && section.items.length > 0) {
      // Continuación sangrada del punto anterior.
      section.items[section.items.length - 1] += ` ${line.trim()}`;
    }
  }
  return releases
    .map((r) => ({ ...r, sections: r.sections.filter((s) => s.items.length > 0) }))
    .filter((r) => r.sections.length > 0);
}

/** Compara `1.2.3` con `1.10.0` numéricamente (sin sufijos de pre-lanzamiento). Negativo si a < b. */
export function compareVersions(a: string, b: string): number {
  const num = (v: string) => v.split("-")[0]!.split(".").map((n) => Number.parseInt(n, 10) || 0);
  const [x, y] = [num(a), num(b)];
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
}

export interface WhatsNewPlan {
  /** Versiones a enseñar (la más nueva primero). */
  show: Release[];
  /** Si hay que guardar la versión actual como vista ya (instalación nueva: no se enseña nada). */
  markSeen: boolean;
}

/**
 * Qué enseñar al abrir la app. `seen` es la última versión cuyas novedades ya se vieron (`null` = nunca se guardó
 * ninguna). Una actualización enseña lo que haya entre la versión vista (sin incluirla) y la actual.
 *
 * Sin versión guardada hay dos casos: una instalación nueva (no se enseña nada: ya tiene el recorrido) y alguien
 * que viene de una versión anterior a esta ventana (`existingUser`: ya usaba la app y no se sabe de qué versión
 * viene, así que se le enseña solo la actual).
 */
export function planWhatsNew(releases: Release[], current: string, seen: string | null, existingUser = false): WhatsNewPlan {
  if (seen === null) {
    const now = existingUser ? releases.find((r) => r.version === current) : undefined;
    return { show: now ? [now] : [], markSeen: true };
  }
  if (compareVersions(seen, current) >= 0) return { show: [], markSeen: false };
  const show = releases.filter((r) => compareVersions(r.version, seen) > 0 && compareVersions(r.version, current) <= 0).slice(0, MAX_RELEASES);
  return { show, markSeen: false };
}

export type InlineSegment = { kind: "text" | "bold" | "code" | "italic"; text: string };

/** Trocea un punto del changelog en texto, **negrita**, `código` y *cursiva*. No genera HTML: es solo una lista de trozos. */
export function parseInline(text: string): InlineSegment[] {
  const out: InlineSegment[] = [];
  const re = /\*\*([^*]+)\*\*|`([^`]+)`|\*([^*]+)\*/g;
  let last = 0;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m.index > last) out.push({ kind: "text", text: text.slice(last, m.index) });
    if (m[1] !== undefined) out.push({ kind: "bold", text: m[1] });
    else if (m[2] !== undefined) out.push({ kind: "code", text: m[2] });
    else out.push({ kind: "italic", text: m[3] ?? "" });
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push({ kind: "text", text: text.slice(last) });
  return out;
}

const SEEN_KEY = "hb.whatsnew.seen";

/** Última versión cuyas novedades se vieron, o `null` si no hay (o no hay almacenamiento). */
export function getSeenVersion(): string | null {
  try {
    return window.localStorage.getItem(SEEN_KEY);
  } catch {
    return null;
  }
}

/** ¿Hay otras preferencias de la interfaz guardadas? Si las hay, no es una instalación nueva. */
export function hasOtherUiFlags(): boolean {
  try {
    for (let i = 0; i < window.localStorage.length; i++) {
      const key = window.localStorage.key(i);
      if (key?.startsWith("hb.") && key !== SEEN_KEY) return true;
    }
  } catch {
    // Sin almacenamiento no se puede saber: se trata como instalación nueva.
  }
  return false;
}

export function setSeenVersion(version: string): void {
  try {
    window.localStorage.setItem(SEEN_KEY, version);
  } catch {
    // Sin almacenamiento: a lo sumo se vuelve a enseñar en el próximo arranque.
  }
}
