import { useTranslation } from "react-i18next";
import { useFlag } from "../../lib/flags";
import { computeSteps, nextStep, progress, type StepId } from "../../lib/gettingStarted";
import type { Page } from "../../lib/nav";
import { Btn } from "../ui";

interface Props {
  connectedEver: boolean;
  rules: number;
  onNavigate: (page: Page) => void;
}

/** Los primeros pasos como una lista con casillas que se marcan solas; desaparece al terminar o al ocultarla. */
export function GettingStarted({ connectedEver, rules, onNavigate }: Props) {
  const { t } = useTranslation();
  const [overlayDone, setOverlayDone] = useFlag("gs.overlay");
  const [testedDone] = useFlag("gs.tested");
  const [hidden, setHidden] = useFlag("gs.hidden");

  const steps = computeSteps({ connectedEver, overlayDone, testedDone, rules });
  const { done, total, complete } = progress(steps);
  const next = nextStep(steps);
  if (hidden || complete) return null;

  const action = (id: StepId): { label: string; run: () => void } | null => {
    switch (id) {
      case "connect":
        return null; // Las tarjetas de conexión están justo encima.
      case "overlay":
        return { label: t("start.overlay.action"), run: () => onNavigate("overlays") };
      case "test":
        return null; // El simulador está más abajo, en esta misma pantalla.
      case "rule":
        return { label: t("start.rule.action"), run: () => onNavigate("rules") };
    }
  };

  return (
    <section className="rounded-xl border border-amber-400/40 bg-brand-900 p-4" aria-labelledby="start-title">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h2 id="start-title" className="text-sm font-semibold text-amber-300">
            {t("start.title")}
          </h2>
          <p className="text-xs text-zinc-400">{t("start.progress", { done, total })}</p>
        </div>
        <button onClick={() => setHidden(true)} className="text-xs text-zinc-400 hover:text-zinc-200">
          {t("start.hide")}
        </button>
      </div>
      <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-brand-950" aria-hidden>
        <div className="h-full bg-gradient-to-r from-amber-400 to-ember-500 transition-all" style={{ width: `${(done / total) * 100}%` }} />
      </div>
      <ol className="mt-3 space-y-2">
        {steps.map((s, i) => {
          const a = action(s.id);
          const isNext = next?.id === s.id;
          return (
            <li key={s.id} className={`flex items-start gap-3 rounded-lg p-2 ${isNext ? "bg-brand-800" : ""}`}>
              <span
                className={`mt-0.5 flex h-5 w-5 flex-none items-center justify-center rounded-full text-xs font-bold ${s.done ? "bg-emerald-500 text-white" : "border border-zinc-500 text-zinc-400"}`}
                aria-hidden
              >
                {s.done ? "✓" : i + 1}
              </span>
              <div className="min-w-0 flex-1">
                <div className={`text-sm font-medium ${s.done ? "text-zinc-400 line-through" : "text-zinc-100"}`}>{t(`start.${s.id}.title`)}</div>
                {!s.done && <div className="text-xs text-zinc-400">{t(`start.${s.id}.hint`)}</div>}
              </div>
              {!s.done && s.id === "overlay" && (
                <Btn onClick={() => setOverlayDone(true)}>{t("start.overlay.done")}</Btn>
              )}
              {!s.done && a && (
                <Btn variant={isNext ? "primary" : "ghost"} onClick={a.run}>
                  {a.label}
                </Btn>
              )}
            </li>
          );
        })}
      </ol>
    </section>
  );
}
