import type { TimerView } from "./types";

/**
 * Tiempo que muestra un timer en marcha, avanzado en local desde la última lectura (`fetchedAt`): el backend solo
 * avisa de los cambios de estado (inicio, pausa, extensiones, fin), no de cada segundo que pasa.
 * Cuenta atrás: baja hasta 0. Cronómetro: sube hasta su tope (si tiene).
 */
export function liveRemaining(tm: TimerView, fetchedAt: number, now: number): number {
  if (tm.status !== "running") return tm.remainingMs;
  const elapsed = Math.max(0, now - fetchedAt);
  if (tm.config.mode === "stopwatch") {
    const shown = tm.remainingMs + elapsed;
    return tm.config.maxSeconds ? Math.min(shown, tm.config.maxSeconds * 1000) : shown;
  }
  return Math.max(0, tm.remainingMs - elapsed);
}
