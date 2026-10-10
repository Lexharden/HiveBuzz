export type Page = "dashboard" | "rules" | "goals" | "interact" | "stats" | "overlays" | "library" | "tts" | "integrations" | "help" | "settings" | "about";

export interface NavGroup {
  /** Clave de i18n en `nav.groups.<key>`; vacía = sin título. */
  key: string | null;
  pages: Page[];
}

/** Menú lateral: las pantallas agrupadas por lo que el streamer quiere hacer. */
export const NAV: NavGroup[] = [
  { key: null, pages: ["dashboard"] },
  { key: "automation", pages: ["rules", "goals"] },
  { key: "community", pages: ["interact", "stats"] },
  { key: "screen", pages: ["overlays", "library", "tts"] },
  { key: "connections", pages: ["integrations"] },
  { key: "app", pages: ["help", "settings", "about"] },
];

export const ICONS: Record<Page, string> = {
  dashboard: "🏠",
  rules: "⚡",
  goals: "🎯",
  interact: "💬",
  stats: "📊",
  overlays: "🖥️",
  library: "🎵",
  tts: "🗣️",
  integrations: "🔌",
  help: "❓",
  settings: "⚙️",
  about: "ℹ️",
};

/** Pantallas que se explican en la ayuda (todas menos la propia ayuda y «Acerca de»). */
export const DOCUMENTED: Page[] = NAV.flatMap((g) => g.pages).filter((p) => p !== "help" && p !== "about");

export const ALL_PAGES: Page[] = NAV.flatMap((g) => g.pages);
