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
import { InstallStatus, useInstall } from "./components/system/useInstall";
import { Tour } from "./components/tour/Tour";
import { TourOffer } from "./components/tour/TourOffer";
import { WhatsNew } from "./components/WhatsNew";
import changelogRaw from "../CHANGELOG.md?raw";
import { getSeenVersion, hasOtherUiFlags, parseChangelog, planWhatsNew, setSeenVersion, type Release } from "./lib/changelog";
import { TtsPage } from "./components/tts/TtsPage";
import { Btn, ToastHost, toast } from "./components/ui";
import i18n from "./i18n";
import { errorMessage } from "./lib/api";
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
  /** Novedades que se están enseñando (tras actualizar, o al pedirlas desde Ayuda). */
  const [whatsNew, setWhatsNew] = useState<Release[] | null>(null);
  const installer = useInstall();

  useEffect(() => {
    void getVersion()
      .then((v) => {
        setVersion(v);
        // Tras actualizar se enseñan las novedades una vez; en una instalación nueva solo se anota la versión.
        const plan = planWhatsNew(parseChangelog(changelogRaw), v, getSeenVersion(), hasOtherUiFlags());
        if (plan.show.length > 0) setWhatsNew(plan.show);
        if (plan.show.length > 0 || plan.markSeen) setSeenVersion(v);
      })
      .catch(() => undefined);
  }, []);

  /** Abre las novedades de la versión instalada a petición del usuario. */
  const showCurrentNotes = () => {
    const current = parseChangelog(changelogRaw).find((r) => version !== null && r.version === version) ?? parseChangelog(changelogRaw)[0];
    if (current) setWhatsNew([current]);
  };

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
            <Btn variant="primary" busy={installer.active} onClick={() => installer.start(() => systemApi.installUpdate()).catch((e: unknown) => toast(errorMessage(e), "error"))}>
              {installer.active ? t("system.update.installing") : t("system.update.install", { version: update.version })}
            </Btn>
          </p>
        )}
        {installer.active && (
          <div className="rounded-lg border border-amber-400/40 bg-amber-400/10 p-3">
            <InstallStatus stage={installer.stage} progress={installer.progress} />
          </div>
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
        {page === "help" && <HelpPage onNavigate={setPage} onStartTour={() => setTouring(true)} onShowWhatsNew={showCurrentNotes} />}
        {page === "settings" && (
          <div className="min-h-0 flex-1 overflow-y-auto">
            {info ? <SettingsPanel info={info} onChanged={refreshInfo} /> : null}
            <div className="mt-4 pb-4">
              <SystemPanel onChanged={refreshInfo} />
            </div>
          </div>
        )}
      </main>

      <ToastHost />
      {whatsNew && <WhatsNew releases={whatsNew} onClose={() => setWhatsNew(null)} />}
      {touring && <Tour page={page} setPage={setPage} onClose={() => setTouring(false)} />}
    </div>
  );
}

export default App;
