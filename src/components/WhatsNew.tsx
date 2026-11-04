import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { parseInline, type Release } from "../lib/changelog";

/** Un punto del changelog con negritas, código y cursivas (se construyen nodos de React, nunca HTML). */
function Inline({ text }: { text: string }) {
  return (
    <>
      {parseInline(text).map((s, i) =>
        s.kind === "bold" ? (
          <strong key={i} className="font-semibold text-zinc-100">{s.text}</strong>
        ) : s.kind === "code" ? (
          <code key={i} className="rounded bg-zinc-950 px-1 text-[0.85em] text-amber-200">{s.text}</code>
        ) : s.kind === "italic" ? (
          <em key={i}>{s.text}</em>
        ) : (
          <span key={i}>{s.text}</span>
        ),
      )}
    </>
  );
}

/** Ventana de «Novedades» tras actualizar (o al pedirla desde Ayuda). Se cierra con el botón, Escape o fuera. */
export function WhatsNew({ releases, onClose }: { releases: Release[]; onClose: () => void }) {
  const { t, i18n } = useTranslation();
  const close = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    close.current?.focus();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const newest = releases[0];
  if (!newest) return null;
  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 p-4" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div role="dialog" aria-modal="true" aria-labelledby="whatsnew-title" className="flex max-h-[85vh] w-full max-w-2xl flex-col rounded-xl border border-brand-700/70 bg-brand-900 shadow-2xl">
        <header className="border-b-2 border-amber-400 px-5 py-4">
          <h2 id="whatsnew-title" className="text-lg font-bold text-white">
            ✨ {t("whatsNew.title")}
          </h2>
          <p className="text-sm text-zinc-400">{t("whatsNew.version", { version: newest.version })}</p>
        </header>

        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-5 py-4">
          {releases.map((r) => (
            <section key={r.version}>
              {releases.length > 1 && (
                <h3 className="mb-2 text-sm font-bold text-amber-300">
                  {r.version} {r.date && <span className="font-normal text-zinc-500">· {r.date}</span>}
                </h3>
              )}
              {r.sections.map((s) => (
                <div key={s.title} className="mb-3">
                  <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-zinc-500">{s.title}</h4>
                  <ul className="space-y-1.5">
                    {s.items.map((item, i) => (
                      <li key={i} className="break-words text-sm leading-snug text-zinc-300">
                        <span aria-hidden className="mr-1.5 text-amber-400">•</span>
                        <Inline text={item} />
                      </li>
                    ))}
                  </ul>
                </div>
              ))}
            </section>
          ))}
        </div>

        <footer className="flex items-center justify-between gap-3 border-t border-brand-700/70 px-5 py-3">
          <span className="text-xs text-zinc-500">{i18n.language !== "es" ? t("whatsNew.spanishOnly") : ""}</span>
          <button ref={close} onClick={onClose} className="rounded-md bg-amber-400 px-4 py-1.5 text-sm font-semibold text-zinc-950 hover:bg-amber-300">
            {t("whatsNew.close")}
          </button>
        </footer>
      </div>
    </div>
  );
}
