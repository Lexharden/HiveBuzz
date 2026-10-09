import { useTranslation } from "react-i18next";
import { statusKey } from "../lib/connectionText";
import { ICONS, NAV, type Page } from "../lib/nav";
import { PLATFORMS, type Platform, type StatusUpdate } from "../lib/types";

const DOT: Record<StatusUpdate["state"], string> = {
  disconnected: "bg-zinc-500",
  waiting_live: "bg-sky-400",
  connected: "bg-emerald-400",
  signature_error: "bg-rose-500",
  reconnecting: "bg-amber-400",
};

interface Props {
  page: Page;
  onNavigate: (p: Page) => void;
  statuses: Record<Platform, StatusUpdate>;
  version: string | null;
}

/** Menú lateral agrupado + un resumen de las conexiones siempre a la vista. */
export function Sidebar({ page, onNavigate, statuses, version }: Props) {
  const { t } = useTranslation();
  return (
    <aside data-tour="sidebar" className="flex w-52 flex-none flex-col bg-brand-800 py-4">
      <div className="flex items-center gap-3 px-4 pb-4">
        <img src="/logo.png" alt="" className="h-12 w-12 flex-none drop-shadow-[0_0_10px_rgba(249,74,32,0.45)]" />
        <div className="min-w-0">
          <h1 className="text-xl font-bold leading-tight text-amber-400">{t("app.name")}</h1>
          {version && (
            <span title={t("app.version")} className="inline-block rounded-full bg-brand-700 px-2 py-0.5 text-[11px] font-semibold text-amber-200">
              v{version}
            </span>
          )}
        </div>
      </div>

      <nav className="min-h-0 flex-1 space-y-3 overflow-y-auto px-2" aria-label={t("nav.label")}>
        {NAV.map((g) => (
          <div key={g.key ?? "top"}>
            {g.key && <div className="px-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-zinc-400">{t(`nav.groups.${g.key}`)}</div>}
            <ul className="space-y-0.5">
              {g.pages.map((p) => {
                const active = p === page;
                return (
                  <li key={p}>
                    <button
                      data-tour={`nav-${p}`}
                      onClick={() => onNavigate(p)}
                      aria-current={active ? "page" : undefined}
                      className={`flex w-full items-center gap-2 rounded-md border-l-4 px-2 py-1.5 text-left text-sm ${
                        active ? "border-amber-400 bg-brand-700 font-semibold text-white" : "border-transparent text-zinc-300 hover:bg-brand-700/50 hover:text-white"
                      }`}
                    >
                      <span aria-hidden className="w-5 text-center">
                        {ICONS[p]}
                      </span>
                      {t(`tabs.${p}`)}
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </nav>

      <button onClick={() => onNavigate("dashboard")} className="mx-2 mt-3 rounded-md bg-brand-900 p-2 text-left hover:bg-brand-950" aria-label={t("nav.connections")}>
        <div className="mb-1 text-[10px] font-semibold uppercase tracking-wider text-zinc-400">{t("nav.connections")}</div>
        {PLATFORMS.map((p) => (
          <div key={p} className="flex items-center gap-2 text-xs text-zinc-200" title={t(statusKey(p, statuses[p]))}>
            <span aria-hidden className={`h-2 w-2 flex-none rounded-full ${DOT[statuses[p].state]} ${statuses[p].state === "reconnecting" || statuses[p].state === "waiting_live" ? "animate-pulse" : ""}`} />
            <span className="font-medium">{t(`platform.${p}`)}</span>
            <span className="truncate text-zinc-400">{t(`conn.short.${statuses[p].state}`)}</span>
          </div>
        ))}
      </button>
    </aside>
  );
}
