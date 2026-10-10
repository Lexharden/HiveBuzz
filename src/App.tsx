import { AboutPage } from "./components/about/AboutPage";
import { getVersion } from "@tauri-apps/api/app";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoalsPage } from "./components/goals/GoalsPage";
import { HelpPage } from "./components/help/HelpPage";
import { HomePage } from "./components/home/HomePage";
import { IntegrationsPage } from "./components/integrations/IntegrationsPage";
import { InteractPage } from "./components/interact/InteractPage";
import { LibraryPage } from "./components/library/LibraryPage";
import { OverlaysPage } from "./components/overlays/OverlaysPage";
import { RulesPage } from "./components/rules/RulesPage";
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { StatsPage } from "./components/stats/StatsPage";
import { SystemPanel } from "./components/system/SystemPanel";
import { Tour } from "./components/tour/Tour";
import { TourOffer } from "./components/tour/TourOffer";
import { TtsPage } from "./components/tts/TtsPage";
import i18n from "./i18n";
import type { Page } from "./lib/nav";
import { systemApi, type UpdateInfo } from "./lib/system";
import { useLive } from "./lib/useLive";

function App() {
  const { t } = useTranslation();
  const { statuses, events, info, error, refreshInfo, clearEvents } = useLive();
  const [page, setPage] = useState<Page>("dashboard");
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [touring, setTouring] = useState(false);

  useEffect(() => {
    void getVersion().then(setVersion).catch(() => undefined);
  }, []);

  // Idioma guardado y, si está configurado, comprobación de actualizaciones al abrir.
  useEffect(() => {
    void systemApi
      .getPrefs()
      .then((p) => {
        if (p.language !== i18n.language) void i18n.changeLanguage(p.language);
        if (p.autoUpdateCheck) {
          void systemApi
            .checkUpdate()
            .then((u) => u.available && setUpdate(u))
            .catch(() => undefined);
        }
      })
      .catch(() => undefined);
  }, []);

  return (
    <div className="flex h-screen">
      <Sidebar page={page} onNavigate={setPage} statuses={statuses} version={version} />

      <main className="flex min-w-0 flex-1 flex-col gap-4 p-5">
        <header className="border-b-2 border-amber-400 pb-3">
          <h2 className="text-xl font-bold text-white">{t(`tabs.${page}`)}</h2>
          <p className="text-sm text-zinc-400">{t(`pageHints.${page}`)}</p>
        </header>

        {error && (
          <p role="alert" className="rounded-lg border border-rose-500/40 bg-rose-500/10 p-3 text-sm text-rose-300">
            {error}
          </p>
        )}
        {update && (
          <p className="flex items-center justify-between rounded-lg border border-amber-400/40 bg-amber-400/10 p-3 text-sm text-amber-200">
            <span>{t("system.update.banner", { version: update.version })}</span>
            <button onClick={() => void systemApi.installUpdate()} className="rounded-md bg-amber-400 px-3 py-1 text-xs font-semibold text-brand-900">
              {t("system.update.install", { version: update.version })}
            </button>
          </p>
        )}

        {page === "dashboard" && !touring && <TourOffer onStart={() => setTouring(true)} />}

        {page === "dashboard" && <HomePage statuses={statuses} events={events} info={info} onClear={clearEvents} onNavigate={setPage} onInfoChanged={refreshInfo} />}
        {page === "rules" && <RulesPage />}
        {page === "overlays" && <OverlaysPage info={info} />}
        {page === "goals" && <GoalsPage />}
        {page === "interact" && <InteractPage />}
        {page === "integrations" && <IntegrationsPage info={info} />}
        {page === "stats" && <StatsPage />}
        {page === "library" && <LibraryPage info={info} />}
        {page === "tts" && <TtsPage />}
        {page === "about" && <AboutPage />}
        {page === "help" && <HelpPage onNavigate={setPage} onStartTour={() => setTouring(true)} />}
        {page === "settings" && (
          <div className="min-h-0 flex-1 overflow-y-auto">
            {info ? <SettingsPanel info={info} onChanged={refreshInfo} /> : null}
            <div className="mt-4 pb-4">
              <SystemPanel onChanged={refreshInfo} />
            </div>
          </div>
        )}
      </main>

      {touring && <Tour page={page} setPage={setPage} onClose={() => setTouring(false)} />}
    </div>
  );
}

export default App;
