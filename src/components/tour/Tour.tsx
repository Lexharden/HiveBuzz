import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { setFlag, useFlag } from "../../lib/flags";
import type { Page } from "../../lib/nav";
import { isSatisfied, placePopover, TOUR, type Rect, type TourStep } from "../../lib/tour";
import { Btn } from "../ui";

const PAD = 6;

function measure(target: string | undefined): Rect | null {
  if (!target) return null;
  const el = document.querySelector<HTMLElement>(`[data-tour="${target}"]`);
  if (!el) return null;
  const r = el.getBoundingClientRect();
  if (r.width === 0 && r.height === 0) return null;
  return { left: r.left - PAD, top: r.top - PAD, width: r.width + PAD * 2, height: r.height + PAD * 2 };
}

/**
 * Recorrido interactivo: oscurece la pantalla, resalta una parte y explica qué es. Los pasos «de hacer»
 * (probar el simulador, abrir Reglas) esperan a que la persona lo haga de verdad. Lo resaltado se puede pulsar.
 */
export function Tour({ page, setPage, onClose }: { page: Page; setPage: (p: Page) => void; onClose: (finished: boolean) => void }) {
  const { t } = useTranslation();
  const [index, setIndex] = useState(0);
  const [rect, setRect] = useState<Rect | null>(null);
  const [pop, setPop] = useState({ w: 360, h: 200 });
  const [view, setView] = useState({ w: window.innerWidth, h: window.innerHeight });
  const popRef = useRef<HTMLDivElement>(null);
  const nextRef = useRef<HTMLButtonElement>(null);
  const [tested] = useFlag("gs.tested");

  const step: TourStep = TOUR[index] ?? TOUR[0]!;
  const last = index === TOUR.length - 1;
  const ready = isSatisfied(step, { page, flag: (k) => (k === "gs.tested" ? tested : false) });

  // Pasos con `navigate`: abren su pantalla al llegar.
  useEffect(() => {
    if (step.navigate && step.page && step.page !== page) setPage(step.page);
    // Solo al cambiar de paso; no si luego la persona navega por su cuenta.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [index]);

  // Un paso «de hacer» con pantalla como condición avanza solo al cumplirse.
  useEffect(() => {
    if (step.wait?.kind === "page" && ready && !last) {
      const id = setTimeout(() => setIndex((i) => Math.min(i + 1, TOUR.length - 1)), 500);
      return () => clearTimeout(id);
    }
  }, [step, ready, last]);

  const locate = useCallback(() => {
    setView({ w: window.innerWidth, h: window.innerHeight });
    setRect(measure(step.target));
  }, [step.target]);

  // El elemento puede tardar en aparecer (cambio de pantalla): se reintenta unos instantes y luego se vigila.
  useEffect(() => {
    let tries = 0;
    const el = () => document.querySelector<HTMLElement>(`[data-tour="${step.target}"]`);
    const find = () => {
      const target = step.target ? el() : null;
      target?.scrollIntoView({ block: "nearest", inline: "nearest" });
      locate();
      if (step.target && !target && tries++ < 30) timer = setTimeout(find, 50);
    };
    let timer = setTimeout(find, 0);
    const watch = setInterval(locate, 400);
    window.addEventListener("resize", locate);
    return () => {
      clearTimeout(timer);
      clearInterval(watch);
      window.removeEventListener("resize", locate);
    };
  }, [index, page, step.target, locate]);

  useLayoutEffect(() => {
    const r = popRef.current?.getBoundingClientRect();
    if (r && (Math.abs(r.width - pop.w) > 1 || Math.abs(r.height - pop.h) > 1)) setPop({ w: r.width, h: r.height });
  });

  useEffect(() => {
    nextRef.current?.focus();
  }, [index]);

  const finish = useCallback(() => {
    setFlag("tour.done", true);
    onClose(true);
  }, [onClose]);
  const go = useCallback(
    (delta: number) => {
      const n = index + delta;
      if (n < 0) return;
      if (n >= TOUR.length) return finish();
      setIndex(n);
    },
    [index, finish],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose(false);
      else if (e.key === "ArrowRight" && ready) go(1);
      else if (e.key === "ArrowLeft") go(-1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [go, onClose, ready]);

  const pos = placePopover(rect, pop, view);
  const title = step.explains ? t(`tabs.${step.explains}`) : step.id === "help" ? t("tabs.help") : t(`tour.steps.${step.id}.title`);
  const body = step.explains ? t(`help.sections.${step.explains}.what`) : t(`tour.steps.${step.id}.body`);
  const waiting = step.wait && !ready;

  return (
    <div className="pointer-events-none fixed inset-0 z-50" aria-live="polite">
      {/* El «agujero» deja ver y pulsar lo resaltado; el resto queda en penumbra. */}
      {rect ? (
        <div
          className="absolute rounded-lg border-2 border-ember-500 transition-all duration-300"
          style={{ left: rect.left, top: rect.top, width: rect.width, height: rect.height, boxShadow: "0 0 0 9999px rgba(0, 15, 33, 0.72)" }}
          aria-hidden
        />
      ) : (
        <div className="absolute inset-0 bg-brand-950/70" aria-hidden />
      )}

      <div
        ref={popRef}
        role="dialog"
        aria-modal="false"
        aria-labelledby="tour-title"
        className="pointer-events-auto absolute w-[22rem] max-w-[calc(100vw-1.5rem)] rounded-2xl border border-ember-500/70 bg-brand-900 p-4 shadow-2xl transition-all duration-300"
        style={{ left: pos.left, top: pos.top }}
      >
        <div className="mb-1 flex items-center justify-between">
          <span className="rounded-full bg-ember-500 px-2 py-0.5 text-[11px] font-bold text-white">{t("tour.counter", { n: index + 1, total: TOUR.length })}</span>
          <button onClick={() => onClose(false)} className="text-xs text-zinc-400 hover:text-zinc-200">
            {t("tour.skip")}
          </button>
        </div>
        <h3 id="tour-title" className="text-base font-bold text-white">
          {title}
        </h3>
        <p className="mt-1 text-sm leading-relaxed text-zinc-200">{body}</p>

        {step.wait && (
          <p className={`mt-2 rounded-md p-2 text-xs ${ready ? "bg-emerald-500/10 text-emerald-300" : "bg-amber-400/10 text-amber-200"}`}>
            {ready ? t("tour.done") : t(`tour.waiting.${step.wait.kind === "flag" ? step.wait.key.replace(".", "_") : "page"}`)}
          </p>
        )}

        <div className="mt-3 flex items-center justify-between gap-2">
          <Btn onClick={() => go(-1)} disabled={index === 0}>
            {t("tour.back")}
          </Btn>
          <button
            ref={nextRef}
            type="button"
            onClick={() => go(1)}
            disabled={Boolean(waiting)}
            className="rounded-md bg-ember-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-ember-400 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {last ? t("tour.finish") : t("tour.next")}
          </button>
        </div>
      </div>
    </div>
  );
}
