export type Page = "dashboard" | "rules" | "goals" | "interact" | "stats" | "overlays" | "library" | "tts" | "integrations" | "help" | "settings";

export interface NavGroup {
  /** Clave de i18n en `nav.groups.<key>`; vacía = sin título. */
  key: string | null;
  pages: Page[];
}

/** Menú lateral: las diez pantallas agrupadas por lo que el streamer quiere hacer. */
export const NAV: NavGroup[] = [
  { key: null, pages: ["dashboard"] },
  { key: "automation", pages: ["rules", "goals"] },
  { key: "community", pages: ["interact", "stats"] },
  { key: "screen", pages: ["overlays", "library", "tts"] },
  { key: "connections", pages: ["integrations"] },
  { key: "app", pages: ["help", "settings"] },
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
};

/** Pantallas que se explican en la ayuda (todas menos la propia ayuda). */
export const DOCUMENTED: Page[] = NAV.flatMap((g) => g.pages).filter((p) => p !== "help");

export const ALL_PAGES: Page[] = NAV.flatMap((g) => g.pages);
