import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import i18n from "../../i18n";
import {
  onUpdateProgress,
  systemApi as api,
  type AppPrefs,
  type BackupSummary,
  type LogEntry,
  type ProfileInfo,
  type UpdateInfo,
  type UpdateProgress,
} from "../../lib/system";
import { Btn, Card, Checkbox, ErrorText, Field, Select, TextInput, useAction } from "../ui";

export function SystemPanel({ onChanged }: { onChanged: () => Promise<void> }) {
  return (
    <div className="space-y-4">
      <PrefsCard onChanged={onChanged} />
      <ProfilesCard />
      <BackupCard />
      <LogsCard />
    </div>
  );
}

function PrefsCard({ onChanged }: { onChanged: () => Promise<void> }) {
  const { t } = useTranslation();
  const [prefs, setPrefs] = useState<AppPrefs | null>(null);
  const [autostart, setAutostart] = useState(false);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const { run, error, busy } = useAction();

  useEffect(() => {
    void api.getPrefs().then(setPrefs);
    void api.getAutostart().then(setAutostart).catch(() => undefined);
    const un = onUpdateProgress(setProgress);
    return () => void un.then((f) => f());
  }, []);

  if (!prefs) return null;
  const save = (next: AppPrefs) =>
    void run(async () => {
      const saved = await api.setPrefs(next);
      setPrefs(saved);
      if (saved.language !== i18n.language) await i18n.changeLanguage(saved.language);
      setMsg(t("system.saved"));
      await onChanged();
    });

  return (
    <Card title={t("system.prefs.title")}>
      <div className="space-y-3">
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("system.prefs.language")} hint={t("system.prefs.languageHint")}>
            <Select value={prefs.language} onChange={(language) => save({ ...prefs, language })} options={[{ value: "es", label: "Español" }, { value: "en", label: "English" }]} />
          </Field>
        </div>
        <div className="grid grid-cols-2 gap-2">
          <Checkbox checked={prefs.closeToTray} onChange={(closeToTray) => save({ ...prefs, closeToTray })} label={t("system.prefs.closeToTray")} />
          <Checkbox checked={prefs.startMinimized} onChange={(startMinimized) => save({ ...prefs, startMinimized })} label={t("system.prefs.startMinimized")} />
          <Checkbox
            checked={autostart}
            onChange={(v) => void run(async () => setAutostart(await api.setAutostart(v)))}
            label={t("system.prefs.autostart")}
          />
        </div>

        <div className="border-t border-zinc-800 pt-3">
          <h3 className="mb-2 text-xs font-semibold text-zinc-300">{t("system.update.title")}</h3>
          <div className="grid grid-cols-[1fr_auto] items-end gap-3">
            <Field label={t("system.update.repo")} hint={t("system.update.repoHint")}>
              <TextInput value={prefs.updateRepo} onChange={(updateRepo) => setPrefs({ ...prefs, updateRepo })} placeholder="Lexharden/HiveBuzz" />
            </Field>
            <Btn disabled={busy} onClick={() => save(prefs)}>
              {t("system.save")}
            </Btn>
          </div>
          <div className="mt-2 flex flex-wrap items-center gap-3">
            <Checkbox checked={prefs.autoUpdateCheck} onChange={(autoUpdateCheck) => save({ ...prefs, autoUpdateCheck })} label={t("system.update.auto")} />
            <Btn
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  setUpdate(await api.checkUpdate());
                })
              }
            >
              {t("system.update.check")}
            </Btn>
            {update && !update.available && <span className="text-xs text-emerald-400">{t("system.update.latest", { version: update.current })}</span>}
            {update?.available && (
              <Btn
                variant="primary"
                disabled={busy}
                onClick={() => void run(async () => api.installUpdate())}
              >
                {t("system.update.install", { version: update.version })}
              </Btn>
            )}
          </div>
          {update?.available && update.notes && <pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap rounded-md bg-zinc-950 p-2 text-xs text-zinc-400">{update.notes}</pre>}
          {progress && <p className="mt-1 text-xs text-zinc-400">{t("system.update.progress", { done: Math.round(progress.downloaded / 1024), total: progress.total ? Math.round(progress.total / 1024) : "?" })}</p>}
        </div>
        {msg && <p className="text-xs text-emerald-400">{msg}</p>}
        <ErrorText error={error} />
      </div>
    </Card>
  );
}

function ProfilesCard() {
  const { t } = useTranslation();
  const [profiles, setProfiles] = useState<ProfileInfo[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const { run, error, busy } = useAction();

  const refresh = useCallback(
    () =>
      void run(async () => {
        setProfiles(await api.listProfiles());
        setActive(await api.activeProfile());
      }),
    [run],
  );
  useEffect(refresh, [refresh]);

  return (
    <Card title={t("system.profiles.title")} hint={t("system.profiles.hint")}>
      <div className="space-y-3">
        <div className="flex gap-2">
          <div className="flex-1">
            <TextInput value={name} onChange={setName} placeholder={t("system.profiles.namePlaceholder")} />
          </div>
          <Btn
            variant="primary"
            disabled={busy || name.trim() === ""}
            onClick={() =>
              void run(async () => {
                await api.saveProfile(null, name);
                setName("");
                setMsg(t("system.profiles.created"));
                refresh();
              })
            }
          >
            {t("system.profiles.saveCurrent")}
          </Btn>
        </div>
        {profiles.length === 0 && <p className="text-xs text-zinc-500">{t("system.profiles.empty")}</p>}
        <ul className="space-y-1">
          {profiles.map((p) => (
            <li key={p.id} className="flex items-center justify-between gap-2 rounded-md border border-zinc-800 px-3 py-2 text-sm">
              <div className="min-w-0">
                <span className="font-semibold">{p.name}</span>
                {active === p.id && <span className="ml-2 rounded bg-amber-400/20 px-1.5 py-0.5 text-xs text-amber-300">{t("system.profiles.active")}</span>}
                <div className="text-xs text-zinc-500">{t("system.profiles.counts", { rules: p.ruleCount, overlays: p.overlayCount })}</div>
              </div>
              <div className="flex flex-none gap-1">
                <Btn
                  variant="primary"
                  disabled={busy}
                  onClick={() => {
                    if (!window.confirm(t("system.profiles.confirmApply", { name: p.name }))) return;
                    void run(async () => {
                      await api.applyProfile(p.id);
                      setMsg(t("system.profiles.applied", { name: p.name }));
                      refresh();
                    });
                  }}
                >
                  {t("system.profiles.apply")}
                </Btn>
                <Btn
                  disabled={busy}
                  onClick={() => {
                    if (!window.confirm(t("system.profiles.confirmOverwrite", { name: p.name }))) return;
                    void run(async () => {
                      await api.saveProfile(p.id, p.name);
                      setMsg(t("system.profiles.updated"));
                      refresh();
                    });
                  }}
                >
                  {t("system.profiles.overwrite")}
                </Btn>
                <Btn
                  variant="danger"
                  disabled={busy}
                  onClick={() => {
                    if (!window.confirm(t("system.profiles.confirmDelete", { name: p.name }))) return;
                    void run(async () => {
                      await api.deleteProfile(p.id);
                      refresh();
                    });
                  }}
                >
                  {t("system.profiles.delete")}
                </Btn>
              </div>
            </li>
          ))}
        </ul>
        {msg && <p className="text-xs text-emerald-400">{msg}</p>}
        <ErrorText error={error} />
      </div>
    </Card>
  );
}

function SummaryText({ s }: { s: BackupSummary }) {
  const { t } = useTranslation();
  return (
    <div className="text-xs text-zinc-400">
      <p>{t("system.backup.summary", { rules: s.rules, goals: s.goals, timers: s.timers, overlays: s.overlays, sounds: s.sounds, media: s.media, profiles: s.profiles })}</p>
      {s.skipped.length > 0 && (
        <details className="mt-1">
          <summary className="cursor-pointer text-amber-300">{t("system.backup.skipped", { n: s.skipped.length })}</summary>
          <ul className="ml-4 list-disc">
            {s.skipped.map((x, i) => (
              <li key={i}>{x}</li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

function BackupCard() {
  const { t } = useTranslation();
  const [exported, setExported] = useState<BackupSummary | null>(null);
  const [staged, setStaged] = useState<BackupSummary | null>(null);
  const [pending, setPending] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const { run, error, busy } = useAction();

  useEffect(() => {
    void api.hasPendingImport().then(setPending);
    void api.takeImportNotice().then(setNotice);
  }, []);

  return (
    <Card title={t("system.backup.title")} hint={t("system.backup.hint")}>
      <div className="space-y-3">
        {notice && (
          <p className={`rounded-md border p-2 text-xs ${notice.startsWith("ok:") ? "border-emerald-500/40 text-emerald-300" : "border-rose-500/40 text-rose-300"}`}>
            {notice.startsWith("ok:") ? t("system.backup.noticeOk", { rules: notice.slice(3) }) : t("system.backup.noticeError", { error: notice.slice(6) })}
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          <Btn
            variant="primary"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                setExported(await api.exportConfig());
              })
            }
          >
            {t("system.backup.export")}
          </Btn>
          <Btn
            disabled={busy}
            onClick={() =>
              void run(async () => {
                const s = await api.importConfig();
                if (s) {
                  setStaged(s);
                  setPending(true);
                }
              })
            }
          >
            {t("system.backup.import")}
          </Btn>
        </div>
        {exported && (
          <div>
            <p className="text-xs text-emerald-400">{t("system.backup.exported")}</p>
            <SummaryText s={exported} />
          </div>
        )}
        {pending && (
          <div className="rounded-md border border-amber-400/40 p-3">
            <p className="text-sm text-amber-300">{t("system.backup.pending")}</p>
            {staged && <SummaryText s={staged} />}
            <div className="mt-2 flex gap-2">
              <Btn variant="primary" disabled={busy} onClick={() => void run(async () => api.restartApp())}>
                {t("system.backup.restart")}
              </Btn>
              <Btn
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await api.cancelPendingImport();
                    setPending(false);
                    setStaged(null);
                  })
                }
              >
                {t("system.backup.cancel")}
              </Btn>
            </div>
          </div>
        )}
        <p className="text-xs text-zinc-500">{t("system.backup.warning")}</p>
        <ErrorText error={error} />
      </div>
    </Card>
  );
}

const LEVELS = ["ERROR", "WARN", "INFO", "DEBUG"] as const;
const levelCls: Record<string, string> = { ERROR: "text-rose-400", WARN: "text-amber-300", INFO: "text-zinc-300", DEBUG: "text-zinc-500" };

function LogsCard() {
  const { t } = useTranslation();
  const [level, setLevel] = useState<(typeof LEVELS)[number]>("INFO");
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [auto, setAuto] = useState(true);
  const box = useRef<HTMLDivElement>(null);
  const { run, error } = useAction();

  const refresh = useCallback(() => void api.getLogs(level, 400).then(setLogs).catch(() => undefined), [level]);
  useEffect(() => {
    refresh();
    if (!auto) return;
    const timer = setInterval(refresh, 2000);
    return () => clearInterval(timer);
  }, [refresh, auto]);
  useEffect(() => {
    if (box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [logs]);

  return (
    <Card
      title={t("system.logs.title")}
      actions={
        <div className="flex items-center gap-2">
          <Select value={level} onChange={setLevel} options={LEVELS.map((l) => ({ value: l, label: l }))} />
          <Checkbox checked={auto} onChange={setAuto} label={t("system.logs.auto")} />
          <Btn onClick={() => void run(async () => { await api.exportLogs(); })}>{t("system.logs.export")}</Btn>
          <Btn onClick={() => void run(async () => { await api.clearLogs(); refresh(); })}>{t("system.logs.clear")}</Btn>
        </div>
      }
    >
      <div ref={box} className="h-64 overflow-y-auto rounded-md bg-zinc-950 p-2 font-mono text-xs">
        {logs.length === 0 && <p className="text-zinc-500">{t("system.logs.empty")}</p>}
        {logs.map((l, i) => (
          <div key={`${l.tsMs}-${i}`} className={levelCls[l.level] ?? "text-zinc-300"}>
            <span className="text-zinc-600">{new Date(l.tsMs).toLocaleTimeString()}</span> {l.level.padEnd(5)} <span className="text-zinc-500">{l.target}</span> {l.message}
          </div>
        ))}
      </div>
      <ErrorText error={error} />
    </Card>
  );
}
