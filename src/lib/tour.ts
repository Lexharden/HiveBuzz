import type { Page } from "./nav";

/** Condición que el usuario debe cumplir para seguir (pasos «de hacer», no solo de leer). */
export type TourWait = { kind: "flag"; key: string } | { kind: "page"; page: Page };

export interface TourStep {
  /** Identificador; los textos están en `tour.steps.<id>` (o se reutilizan los de la ayuda). */
  id: string;
  /** Pantalla que debe estar abierta para mostrar el paso (se abre sola si `navigate`). */
  page?: Page;
  /** Elemento a resaltar: `data-tour="<target>"`. Sin él, el paso va centrado. */
  target?: string;
  /** Abre `page` automáticamente al llegar al paso. */
  navigate?: boolean;
  wait?: TourWait;
  /** Los textos salen de la ayuda de esta pantalla (`help.sections.<page>.what`). */
  explains?: Page;
}

/** El recorrido básico: conectar → probar → conocer cada parte del menú. */
export const TOUR: TourStep[] = [
  { id: "welcome", page: "dashboard", navigate: true },
  { id: "connections", page: "dashboard", target: "connections" },
  { id: "simulate", page: "dashboard", target: "simulator", wait: { kind: "flag", key: "gs.tested" } },
  { id: "feed", page: "dashboard", target: "feed" },
  { id: "menu", target: "sidebar" },
  { id: "goRules", target: "nav-rules", wait: { kind: "page", page: "rules" } },
  { id: "rules", page: "rules", target: "nav-rules", explains: "rules" },
  { id: "goals", page: "goals", target: "nav-goals", navigate: true, explains: "goals" },
  { id: "interact", page: "interact", target: "nav-interact", navigate: true, explains: "interact" },
  { id: "stats", page: "stats", target: "nav-stats", navigate: true, explains: "stats" },
  { id: "overlays", page: "overlays", target: "nav-overlays", navigate: true, explains: "overlays" },
  { id: "library", page: "library", target: "nav-library", navigate: true, explains: "library" },
  { id: "tts", page: "tts", target: "nav-tts", navigate: true, explains: "tts" },
  { id: "integrations", page: "integrations", target: "nav-integrations", navigate: true, explains: "integrations" },
  { id: "help", page: "help", target: "nav-help", navigate: true },
  { id: "settings", page: "settings", target: "nav-settings", navigate: true, explains: "settings" },
  { id: "finish", page: "dashboard", navigate: true },
];

export interface TourContext {
  page: Page;
  flag: (key: string) => boolean;
}

/** ¿Ya hizo lo que pide este paso? Un paso sin condición siempre está listo. */
export function isSatisfied(step: TourStep, ctx: TourContext): boolean {
  const w = step.wait;
  if (!w) return true;
  return w.kind === "flag" ? ctx.flag(w.key) : ctx.page === w.page;
}

export interface Rect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface Size {
  w: number;
  h: number;
}

/**
 * Dónde poner el cuadro de texto junto al elemento resaltado: a la derecha si cabe (el menú está a la
 * izquierda), si no debajo, si no encima, y si nada cabe, centrado. Siempre dentro de la ventana.
 */
export function placePopover(target: Rect | null, pop: Size, view: Size, gap = 14, margin = 12): { left: number; top: number } {
  const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(v, Math.max(lo, hi)));
  const centered = { left: clamp((view.w - pop.w) / 2, margin, view.w - pop.w - margin), top: clamp((view.h - pop.h) / 2, margin, view.h - pop.h - margin) };
  if (!target) return centered;
  const fitsRight = target.left + target.width + gap + pop.w + margin <= view.w;
  const fitsBelow = target.top + target.height + gap + pop.h + margin <= view.h;
  const fitsAbove = target.top - gap - pop.h - margin >= 0;
  if (fitsRight) return { left: target.left + target.width + gap, top: clamp(target.top, margin, view.h - pop.h - margin) };
  if (fitsBelow) return { left: clamp(target.left, margin, view.w - pop.w - margin), top: target.top + target.height + gap };
  if (fitsAbove) return { left: clamp(target.left, margin, view.w - pop.w - margin), top: target.top - gap - pop.h };
  return centered;
}
