import { useTranslation } from "react-i18next";
import { useFlag } from "../../lib/flags";

/** Invitación (una sola vez) a hacer el recorrido. Cualquiera de los dos botones la oculta para siempre. */
export function TourOffer({ onStart }: { onStart: () => void }) {
  const { t } = useTranslation();
  const [seen, setSeen] = useFlag("tour.seen");
  if (seen) return null;
  return (
    <section className="flex flex-wrap items-center gap-4 rounded-xl border border-ember-500/60 bg-ember-500/10 p-4" aria-label={t("tour.offer.title")}>
      <span aria-hidden className="text-3xl">
        👋
      </span>
      <div className="min-w-0 flex-1">
        <h2 className="text-sm font-bold text-white">{t("tour.offer.title")}</h2>
        <p className="text-xs text-zinc-300">{t("tour.offer.body")}</p>
      </div>
      <div className="flex gap-2">
        <button
          onClick={() => {
            setSeen(true);
            onStart();
          }}
          className="rounded-md bg-ember-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-ember-400"
        >
          {t("tour.offer.start")}
        </button>
        <button onClick={() => setSeen(true)} className="rounded-md px-3 py-1.5 text-xs text-zinc-300 hover:bg-brand-800">
          {t("tour.offer.later")}
        </button>
      </div>
    </section>
  );
}
