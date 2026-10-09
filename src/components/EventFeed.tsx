import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { countByPlatform, filterEvents, platformOf, type FeedFilter } from "../lib/feedFilter";
import type { LiveEvent } from "../lib/types";
import { PlatformBadge } from "./PlatformBadge";

const BORDER: Record<string, string> = {
  gift: "border-pink-400",
  chat: "border-sky-400",
  like: "border-rose-400",
  follow: "border-emerald-400",
  subscribe: "border-emerald-400",
  share: "border-violet-400",
};

type T = (k: string, o?: Record<string, unknown>) => string;

function describe(ev: LiveEvent, t: T): string {
  switch (ev.type) {
    case "gift": {
      const g = ev.gift;
      // En Twitch los regalos son bits; en TikTok, monedas.
      const key = platformOf(ev) === "twitch" ? "feed.bits" : "feed.gift";
      return t(key, { count: g?.count ?? 1, name: g?.name ?? "?", coins: g?.coins ?? 0 });
    }
    case "chat":
      return ev.chat?.text ?? "";
    case "like":
      return t("feed.like", { count: ev.like?.count ?? 0 });
    default:
      return t(`feed.${ev.type}`);
  }
}

export function EventFeed({ events, onClear }: { events: LiveEvent[]; onClear: () => void }) {
  const { t } = useTranslation();
  const endRef = useRef<HTMLDivElement>(null);
  const [filter, setFilter] = useState<FeedFilter>("all");
  const counts = useMemo(() => countByPlatform(events), [events]);
  const shown = useMemo(() => filterEvents(events, filter), [events, filter]);
  // Con una sola plataforma en uso los filtros no aportan: no se muestran.
  const showFilters = counts.tiktok > 0 && counts.twitch > 0;

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end" });
  }, [shown.length]);

  const chip = (value: FeedFilter, label: string, n: number) => (
    <button
      key={value}
      onClick={() => setFilter(value)}
      aria-pressed={filter === value}
      className={`rounded-full px-2.5 py-0.5 text-xs ${filter === value ? "bg-amber-400 font-semibold text-brand-900" : "bg-brand-800 text-zinc-300 hover:bg-brand-700"}`}
    >
      {label} <span className="opacity-70">{n}</span>
    </button>
  );

  return (
    <section className="flex min-h-0 flex-1 flex-col rounded-xl border border-brand-700/70 bg-brand-900">
      <header className="flex flex-wrap items-center justify-between gap-2 border-b border-brand-700/60 px-4 py-2">
        <h2 className="text-sm font-semibold text-zinc-300">{t("feed.title")}</h2>
        <div className="flex items-center gap-3">
          {showFilters && (
            <div className="flex gap-1" role="group" aria-label={t("feed.filter")}>
              {chip("all", t("feed.all"), events.length)}
              {chip("tiktok", t("platform.tiktok"), counts.tiktok)}
              {chip("twitch", t("platform.twitch"), counts.twitch)}
            </div>
          )}
          <button onClick={onClear} className="text-xs text-zinc-400 hover:text-zinc-200">
            {t("feed.clear")}
          </button>
        </div>
      </header>
      <div className="min-h-0 flex-1 space-y-1 overflow-y-auto p-3">
        {shown.length === 0 && <p className="py-8 text-center text-sm text-zinc-500">{events.length === 0 ? t("feed.empty") : t("feed.emptyFilter")}</p>}
        {shown.map((ev) => (
          // El texto de TikTok y de Twitch se renderiza como texto de React (escapado), nunca como HTML.
          <div key={ev.id} className={`rounded-md border-l-4 bg-brand-950 px-3 py-1.5 text-sm ${BORDER[ev.type] ?? "border-zinc-600"}`}>
            {showFilters && <PlatformBadge platform={platformOf(ev)} className="mr-2 align-middle" />}
            <span className="font-semibold text-amber-300">{ev.user.nickname || ev.user.uniqueId}</span>
            {ev.type === "chat" ? ": " : " "}
            <span className="break-words text-zinc-200">{describe(ev, t)}</span>
          </div>
        ))}
        <div ref={endRef} />
      </div>
    </section>
  );
}
