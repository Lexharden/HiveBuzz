import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, onInstallProgress } from "../../lib/api";
import type { InstallProgress, Role, TtsConfig, TtsStatus, VoiceInfo, VoiceMode } from "../../lib/types";
import { Btn, Card, Checkbox, ErrorText, Field, NumberInput, Select, TextInput, useAction } from "../ui";

const ROLES: Role[] = ["moderator", "subscriber", "follower"];

export function TtsPage() {
  const { t } = useTranslation();
  const [config, setConfig] = useState<TtsConfig | null>(null);
  const [voices, setVoices] = useState<VoiceInfo[]>([]);
  const [status, setStatus] = useState<TtsStatus | null>(null);
  const [progress, setProgress] = useState<InstallProgress | null>(null);
  const [saved, setSaved] = useState(false);
  const [testText, setTestText] = useState("Hola, esta es una prueba de voz.");
  const [testVoice, setTestVoice] = useState("");
  const { run, error, busy } = useAction();
  const alive = useRef(true);

  const reloadVoices = useCallback(async () => {
    const [v, s] = await Promise.all([api.listTtsVoices(), api.getTtsStatus()]);
    if (alive.current) {
      setVoices(v);
      setStatus(s);
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    let off: (() => void) | undefined;
    let disposed = false;
    void run(async () => {
      setConfig(await api.getTtsConfig());
      await reloadVoices();
    });
    void onInstallProgress((p) => alive.current && setProgress(p)).then((u) => (disposed ? u() : (off = u)));
    return () => {
      disposed = true;
      alive.current = false;
      off?.();
    };
  }, [reloadVoices, run]);

  if (!config) return <ErrorText error={error} />;

  const patch = (p: Partial<TtsConfig>) => {
    setSaved(false);
    setConfig((c) => (c ? { ...c, ...p } : c));
  };
  const patchFilters = (p: Partial<TtsConfig["filters"]>) => patch({ filters: { ...config.filters, ...p } });
  const toggleRole = (r: Role, on: boolean) => patch({ rolesAny: on ? [...config.rolesAny, r] : config.rolesAny.filter((x) => x !== r) });
  const voiceOptions = [{ value: "", label: t("tts.noVoice") }, ...voices.map((v) => ({ value: v.id, label: `${v.name} (${v.engine}${v.lang ? ` · ${v.lang}` : ""})` }))];
  const pct = progress?.total ? Math.min(100, Math.round((progress.done / progress.total) * 100)) : null;

  const install = (fn: () => Promise<void>) =>
    void run(async () => {
      setProgress(null);
      try {
        await fn();
        await reloadVoices();
      } finally {
        setProgress(null);
      }
    });

  return (
    <div className="min-h-0 flex-1 space-y-4 overflow-y-auto">
      <Card title={t("tts.engines")} hint={t("tts.enginesHint")}>
        <div className="flex flex-wrap items-center gap-3">
          <span className={`text-xs ${status?.piperInstalled ? "text-emerald-400" : "text-amber-300"}`}>
            Piper: {status?.piperInstalled ? t("tts.installed") : t("tts.notInstalled")}
          </span>
          {status?.canInstallPiper && !status.piperInstalled && (
            <Btn variant="primary" disabled={busy} onClick={() => install(api.installPiper)}>
              {t("tts.installPiper")}
            </Btn>
          )}
          {!status?.canInstallPiper && !status?.piperInstalled && <span className="text-xs text-zinc-500">{t("tts.piperManual")}</span>}
          <span className="text-xs text-zinc-500">{t("tts.voiceCount", { n: voices.length })}</span>
        </div>
        {progress && (
          <div className="mt-3">
            <div className="mb-1 text-xs text-zinc-400">
              {progress.stage}
              {pct !== null && ` · ${pct}%`}
            </div>
            <div className="h-2 overflow-hidden rounded bg-zinc-800">
              <div className="h-full bg-amber-400 transition-all" style={{ width: `${pct ?? 100}%`, opacity: pct === null ? 0.4 : 1 }} />
            </div>
          </div>
        )}
        {status?.piperInstalled && (
          <ul className="mt-3 grid grid-cols-1 gap-2 md:grid-cols-2">
            {status.catalog.map((v) => (
              <li key={v.id} className="flex items-center justify-between gap-2 rounded-md border border-zinc-800 px-3 py-2 text-xs">
                <span className="min-w-0 truncate" title={v.label}>
                  {v.label} <span className="text-zinc-500">· {v.approxMb} MB</span>
                </span>
                {v.installed ? (
                  <span className="text-emerald-400">{t("tts.installed")}</span>
                ) : (
                  <Btn disabled={busy} onClick={() => install(() => api.installPiperVoice(v.id))}>{t("tts.download")}</Btn>
                )}
              </li>
            ))}
          </ul>
        )}
      </Card>

      <Card title={t("tts.test")}>
        <div className="grid grid-cols-3 gap-2">
          <Field label={t("tts.testText")} className="col-span-2">
            <TextInput value={testText} onChange={setTestText} />
          </Field>
          <Field label={t("tts.voice")}>
            <Select value={testVoice} onChange={setTestVoice} options={voiceOptions} />
          </Field>
        </div>
        <div className="mt-3 flex gap-2">
          <Btn variant="primary" disabled={busy || !testText.trim()} onClick={() => void run(() => api.ttsPreview(testText, testVoice || undefined))}>
            ▶ {t("tts.speak")}
          </Btn>
          <Btn onClick={() => void api.ttsSkip()}>⏭ {t("tts.skip")}</Btn>
        </div>
      </Card>

      <Card title={t("tts.chatReader")} hint={t("tts.chatReaderHint")}>
        <div className="space-y-3">
          <Checkbox checked={config.enabled} onChange={(enabled) => patch({ enabled })} label={t("tts.enabled")} />
          <div className="grid grid-cols-2 gap-3">
            <Field label={t("tts.command")} hint={t("tts.commandHint")}>
              <TextInput value={config.command ?? ""} onChange={(v) => patch({ command: v.trim() === "" ? null : v })} placeholder="tts" />
            </Field>
            <Field label={t("tts.template")} hint={t("tts.templateHint")}>
              <TextInput value={config.template} onChange={(template) => patch({ template })} />
            </Field>
          </div>
          <Field label={t("tts.roles")} hint={t("tts.rolesHint")}>
            <div className="flex gap-4">
              {ROLES.map((r) => (
                <Checkbox key={r} checked={config.rolesAny.includes(r)} onChange={(on) => toggleRole(r, on)} label={t(`role.${r}`)} />
              ))}
            </div>
          </Field>
          <div className="grid grid-cols-4 gap-3">
            <Field label={t("cond.minTeam")}>
              <NumberInput value={config.minTeamLevel} min={0} onChange={(v) => patch({ minTeamLevel: v })} />
            </Field>
            <Field label={t("cond.minGifter")}>
              <NumberInput value={config.minGifterLevel} min={0} onChange={(v) => patch({ minGifterLevel: v })} />
            </Field>
            <Field label={t("tts.recentDonors")} hint={t("tts.recentDonorsHint")}>
              <NumberInput value={config.recentDonorsMinutes} min={1} onChange={(v) => patch({ recentDonorsMinutes: v })} />
            </Field>
            <Field label={t("tts.userCooldown")}>
              <NumberInput value={config.userCooldownMs / 1000} min={0} onChange={(v) => patch({ userCooldownMs: Math.round((v ?? 0) * 1000) })} />
            </Field>
          </div>
          <Field label={t("tts.ignoreUsers")} hint={t("tts.ignoreUsersHint")}>
            <TextInput
              value={config.ignoreUsers.join(", ")}
              onChange={(v) => patch({ ignoreUsers: v.split(",").map((u) => u.trim()).filter(Boolean) })}
            />
          </Field>
          <Checkbox checked={config.ignoreBangCommands} onChange={(ignoreBangCommands) => patch({ ignoreBangCommands })} label={t("tts.ignoreBang")} />
        </div>
      </Card>

      <Card title={t("tts.voices")}>
        <div className="space-y-3">
          <div className="grid grid-cols-3 gap-3">
            <Field label={t("tts.voiceMode")}>
              <Select<VoiceMode>
                value={config.voiceMode}
                onChange={(voiceMode) => patch({ voiceMode })}
                options={[
                  { value: "single", label: t("tts.mode.single") },
                  { value: "byRole", label: t("tts.mode.byRole") },
                  { value: "randomPerUser", label: t("tts.mode.randomPerUser") },
                ]}
              />
            </Field>
            <Field label={t("tts.defaultVoice")}>
              <Select value={config.defaultVoice ?? ""} onChange={(v) => patch({ defaultVoice: v || null })} options={voiceOptions} />
            </Field>
            <div className="grid grid-cols-2 gap-2">
              <Field label={t("tts.rate")}>
                <NumberInput value={config.rate} min={0.5} max={2} step={0.1} onChange={(v) => patch({ rate: v ?? 1 })} />
              </Field>
              <Field label={t("tts.volume")}>
                <NumberInput value={config.volume} min={0} max={100} onChange={(v) => patch({ volume: v ?? 100 })} />
              </Field>
            </div>
          </div>
          {config.voiceMode === "byRole" && (
            <div className="grid grid-cols-3 gap-3">
              {ROLES.map((r) => (
                <Field key={r} label={t(`role.${r}`)}>
                  <Select
                    value={config.roleVoices[r] ?? ""}
                    onChange={(v) => patch({ roleVoices: { ...config.roleVoices, [r]: v || null } })}
                    options={voiceOptions}
                  />
                </Field>
              ))}
            </div>
          )}
          {config.voiceMode === "randomPerUser" && (
            <Field label={t("tts.randomPool")} hint={t("tts.randomPoolHint")}>
              <div className="grid max-h-36 grid-cols-2 gap-1 overflow-y-auto rounded-md border border-zinc-800 p-2">
                {voices.map((v) => (
                  <Checkbox
                    key={v.id}
                    checked={config.randomVoices.includes(v.id)}
                    onChange={(on) => patch({ randomVoices: on ? [...config.randomVoices, v.id] : config.randomVoices.filter((x) => x !== v.id) })}
                    label={`${v.name} (${v.engine})`}
                  />
                ))}
              </div>
            </Field>
          )}
        </div>
      </Card>

      <Card title={t("tts.filtersTitle")} hint={t("tts.filtersHint")}>
        <div className="space-y-3">
          <div className="flex flex-wrap gap-5">
            <Checkbox checked={config.filters.skipLinks} onChange={(skipLinks) => patchFilters({ skipLinks })} label={t("tts.skipLinks")} />
            <Checkbox checked={config.filters.stripEmojis} onChange={(stripEmojis) => patchFilters({ stripEmojis })} label={t("tts.stripEmojis")} />
            <Checkbox checked={config.filters.profanityEnabled} onChange={(profanityEnabled) => patchFilters({ profanityEnabled })} label={t("tts.profanity")} />
          </div>
          <div className="grid grid-cols-3 gap-3">
            <Field label={t("tts.maxChars")}>
              <NumberInput value={config.filters.maxChars} min={10} max={1000} onChange={(v) => patchFilters({ maxChars: v ?? 200 })} />
            </Field>
            <Field label={t("tts.maxRepeat")} hint={t("tts.maxRepeatHint")}>
              <NumberInput value={config.filters.maxRepeat} min={0} max={20} onChange={(v) => patchFilters({ maxRepeat: v ?? 3 })} />
            </Field>
            <Field label={t("tts.profanityMode")}>
              <Select
                value={config.filters.profanityMode}
                onChange={(profanityMode) => patchFilters({ profanityMode })}
                options={[
                  { value: "skip", label: t("tts.profanitySkip") },
                  { value: "censor", label: t("tts.profanityCensor") },
                ]}
              />
            </Field>
          </div>
          {config.filters.profanityEnabled && (
            <Field label={t("tts.profanityWords")} hint={t("tts.profanityWordsHint")}>
              <textarea
                rows={3}
                value={config.filters.profanityWords.join(", ")}
                onChange={(e) => patchFilters({ profanityWords: e.target.value.split(/[,\n]/).map((w) => w.trim()).filter(Boolean) })}
                className="w-full rounded-md border border-zinc-700 bg-zinc-950 p-2 text-sm outline-none focus:border-amber-400"
              />
            </Field>
          )}
        </div>
      </Card>

      <div className="flex items-center justify-end gap-3 pb-2">
        <ErrorText error={error} />
        {saved && <span className="text-xs text-emerald-400">{t("tts.saved")}</span>}
        <Btn
          variant="primary"
          disabled={busy}
          onClick={() =>
            void run(async () => {
              setConfig(await api.setTtsConfig(config));
              setSaved(true);
              await reloadVoices();
            })
          }
        >
          {t("tts.save")}
        </Btn>
      </div>
    </div>
  );
}
