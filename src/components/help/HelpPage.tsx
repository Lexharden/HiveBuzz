import { useTranslation } from "react-i18next";
import { DOCUMENTED, ICONS, type Page } from "../../lib/nav";
import { Btn, Card } from "../ui";

const GLOSSARY = ["event", "rule", "trigger", "action", "overlay", "session", "points", "tts", "cooldown"] as const;
const FAQ = ["obs", "bot", "spotify", "safe", "test", "update"] as const;
const ITEMS = [1, 2, 3, 4] as const;

/** Ayuda: el recorrido, qué hay en cada parte del menú, un glosario y respuestas rápidas. */
export function HelpPage({ onNavigate, onStartTour }: { onNavigate: (p: Page) => void; onStartTour: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="min-h-0 flex-1 space-y-4 overflow-y-auto pb-4">
      <section className="flex flex-wrap items-center gap-4 rounded-xl border border-ember-500/60 bg-ember-500/10 p-4">
        <img src="/logo.png" alt="" className="h-16 w-16 flex-none" />
        <div className="min-w-0 flex-1">
          <h2 className="text-base font-bold text-white">{t("help.tour.title")}</h2>
          <p className="text-sm text-zinc-300">{t("help.tour.body")}</p>
        </div>
        <Btn variant="accent" className="px-4 py-2 text-sm" onClick={onStartTour}>
          ▶ {t("help.tour.start")}
        </Btn>
      </section>

      <Card title={t("help.menu.title")} hint={t("help.menu.hint")}>
        <div className="grid gap-3 lg:grid-cols-2">
          {DOCUMENTED.map((p) => (
            <article key={p} className="rounded-lg border border-brand-700/60 bg-brand-950 p-3">
              <h3 className="flex items-center gap-2 text-sm font-bold text-amber-300">
                <span aria-hidden>{ICONS[p]}</span>
                {t(`tabs.${p}`)}
              </h3>
              <p className="mt-1 text-sm text-zinc-200">{t(`help.sections.${p}.what`)}</p>
              <ul className="mt-2 list-disc space-y-1 pl-5 text-xs text-zinc-400">
                {ITEMS.map((n) => (
                  <li key={n}>{t(`help.sections.${p}.i${n}`)}</li>
                ))}
              </ul>
              <Btn className="mt-3" onClick={() => onNavigate(p)}>
                {t("help.menu.go", { name: t(`tabs.${p}`) })}
              </Btn>
            </article>
          ))}
        </div>
      </Card>

      <Card title={t("help.glossary.title")} hint={t("help.glossary.hint")}>
        <dl className="grid gap-x-6 gap-y-2 text-sm lg:grid-cols-2">
          {GLOSSARY.map((g) => (
            <div key={g}>
              <dt className="font-semibold text-ember-300">{t(`help.glossary.${g}.term`)}</dt>
              <dd className="text-zinc-300">{t(`help.glossary.${g}.def`)}</dd>
            </div>
          ))}
        </dl>
      </Card>

      <Card title={t("help.faq.title")}>
        <div className="space-y-2">
          {FAQ.map((f) => (
            <details key={f} className="rounded-md border border-brand-700/60 bg-brand-950 px-3 py-2">
              <summary className="cursor-pointer text-sm font-medium text-zinc-100">{t(`help.faq.${f}.q`)}</summary>
              <p className="mt-2 text-sm text-zinc-300">{t(`help.faq.${f}.a`)}</p>
            </details>
          ))}
        </div>
      </Card>
    </div>
  );
}
