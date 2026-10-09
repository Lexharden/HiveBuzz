export type StepId = "connect" | "overlay" | "test" | "rule";

export interface StepInput {
  /** Alguna plataforma está conectada o se conectó alguna vez. */
  connectedEver: boolean;
  /** El usuario marcó que ya añadió un overlay a OBS (no se puede saber desde la app). */
  overlayDone: boolean;
  /** Ya probó una alerta con el simulador. */
  testedDone: boolean;
  /** Reglas creadas. */
  rules: number;
}

export interface Step {
  id: StepId;
  done: boolean;
}

/** Los cuatro primeros pasos, en orden, con su estado. */
export function computeSteps(i: StepInput): Step[] {
  return [
    { id: "connect", done: i.connectedEver },
    { id: "overlay", done: i.overlayDone },
    { id: "test", done: i.testedDone },
    { id: "rule", done: i.rules > 0 },
  ];
}

export const progress = (steps: Step[]): { done: number; total: number; complete: boolean } => {
  const done = steps.filter((s) => s.done).length;
  return { done, total: steps.length, complete: done === steps.length };
};

/** El primer paso pendiente (el «siguiente paso obvio»). */
export const nextStep = (steps: Step[]): Step | undefined => steps.find((s) => !s.done);
