import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { onUpdateProgress, systemApi as api, type ReleaseInfo, type UpdateProgress } from "../../lib/system";
import { Btn, Card, ErrorText, useAction } from "../ui";

const RELEASES_PAGE = "https://github.com/Lexharden/HiveBuzz/releases";

const BADGE: Record<ReleaseInfo["relation"], string> = {
  newer: "bg-amber-400/15 text-amber-300",
  current: "bg-emerald-500/15 text-emerald-300",
  older: "bg-zinc-700/50 text-zinc-400",
  unknown: "bg-zinc-700/50 text-zinc-400",
};

/** Versiones publicadas en GitHub: instalar la última o elegir cualquier otra («Comprobar al abrir» está en Preferencias). */
export function UpdatesCard() {
  const { t, i18n } = useTranslation();
  const [releases, setReleases] = useState<ReleaseInfo[] | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const [installing, setInstalling] = useState<string | null>(null);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const { run, error, busy } = useAction();

  const load = useCallback(() => void run(async () => setReleases(await api.listReleases())), [run]);

  useEffect(() => {
    load();
    const un = onUpdateProgress(setProgress);
    return () => void un.then((f) => f());
  }, [load]);

  const install = (r: ReleaseInfo) => {
    if (r.relation === "older" && !window.confirm(t("system.update.confirmOlder", { version: r.version }))) return;
    void run(async () => {
      setInstalling(r.tag);
      setProgress(null);
      try {
        await api.prepareRelease(r.tag);
        await api.installUpdate();
      } finally {
        setInstalling(null);
      }
    });
  };

  const date = (iso: string | null) => (iso ? new Date(iso).toLocaleDateString(i18n.language, { year: "numeric", month: "short", day: "numeric" }) : "");
  const newest = releases?.find((r) => r.relation === "newer" && r.installable && !r.prerelease);

  return (
    <Card
      title={t("system.update.title")}
      hint={t("system.update.hint")}
      actions={
        <div className="flex gap-2">
          <Btn onClick={() => void openUrl(RELEASES_PAGE)}>{t("system.update.openGithub")}</Btn>
          <Btn disabled={busy} onClick={load}>
            ↻ {t("system.update.check")}
          </Btn>
        </div>
      }
    >
      <div className="space-y-3">
        {releases && (
          <p className={`text-xs ${newest ? "text-amber-300" : "text-emerald-400"}`}>
            {newest ? t("system.update.newer", { version: newest.version }) : releases.length > 0 ? t("system.update.upToDate") : null}
          </p>
        )}

        {releases?.length === 0 && <p className="py-4 text-center text-sm text-zinc-500">{t("system.update.none")}</p>}

        {releases && releases.length > 0 && (
          <ul className="max-h-96 space-y-2 overflow-y-auto pr-1">
            {releases.map((r) => (
              <li key={r.tag} className="rounded-lg border border-zinc-800 bg-zinc-950/50 px-3 py-2">
                <div className="flex items-center gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-sm font-semibold">v{r.version}</span>
                      {r.relation !== "unknown" && <span className={`rounded px-1.5 text-[10px] ${BADGE[r.relation]}`}>{t(`system.update.rel.${r.relation}`)}</span>}
                      {r.prerelease && <span className="rounded bg-sky-500/15 px-1.5 text-[10px] text-sky-300">{t("system.update.prerelease")}</span>}
                      {!r.installable && <span className="rounded bg-zinc-700/50 px-1.5 text-[10px] text-zinc-400">{t("system.update.manualOnly")}</span>}
                    </div>
                    <div className="truncate text-xs text-zinc-500">
                      {r.name !== r.tag && r.name !== `v${r.version}` ? `${r.name} · ` : ""}
                      {date(r.publishedAt)}
                    </div>
                  </div>
                  {r.notes.trim() && <Btn onClick={() => setOpen(open === r.tag ? null : r.tag)}>{t(open === r.tag ? "system.update.hideNotes" : "system.update.notes")}</Btn>}
                  {r.relation !== "current" &&
                    (r.installable ? (
                      <Btn variant={r.relation === "newer" ? "primary" : "ghost"} disabled={busy} onClick={() => install(r)}>
                        {installing === r.tag ? t("system.update.installing") : t("system.update.installThis")}
                      </Btn>
                    ) : (
                      <Btn onClick={() => void openUrl(r.url)}>{t("system.update.download")}</Btn>
                    ))}
                </div>
                {open === r.tag && <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap rounded-md bg-zinc-950 p-2 text-xs text-zinc-400">{r.notes}</pre>}
                {installing === r.tag && progress && (
                  <p className="mt-1 text-xs text-zinc-400">
                    {t("system.update.progress", { done: Math.round(progress.downloaded / 1024), total: progress.total ? Math.round(progress.total / 1024) : "?" })}
                  </p>
                )}
              </li>
            ))}
          </ul>
        )}
        <ErrorText error={error} />
      </div>
    </Card>
  );
}
