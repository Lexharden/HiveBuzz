import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { systemApi as api, type SpotifyConfig, type SpotifyStatus } from "../../lib/system";
import { Btn, Card, Checkbox, ErrorText, Field, NumberInput, Select, TextInput, useAction } from "../ui";

const lines = (v: string[]): string => v.join(", ");
const parseList = (raw: string): string[] =>
  raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);

export function SpotifySection() {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<SpotifyConfig | null>(null);
  const [status, setStatus] = useState<SpotifyStatus | null>(null);
  const [query, setQuery] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const { run, error, busy } = useAction();

  const refreshStatus = useCallback(() => void api.spotifyStatus().then(setStatus).catch(() => undefined), []);
  useEffect(() => {
    void api.spotifyGetConfig().then(setCfg);
    refreshStatus();
    // Tras autorizar en el navegador la app no recibe aviso: se consulta cada pocos segundos.
    const timer = setInterval(refreshStatus, 2000);
    return () => clearInterval(timer);
  }, [refreshStatus]);
  if (!cfg) return null;
  const song = cfg.song;
  const setSong = (patch: Partial<typeof song>) => setCfg({ ...cfg, song: { ...song, ...patch } });
  const save = (next: SpotifyConfig) =>
    run(async () => {
      setCfg(await api.spotifySetConfig(next));
      refreshStatus();
    });
  const connected = status?.connected ?? false;

  return (
    <Card title={t("spotify.title")} hint={t("spotify.hintEasy")}>
      <div className="space-y-4">
        {/* Paso único para el usuario normal */}
        {!connected ? (
          <div className="space-y-2">
            <Btn
              variant="primary"
              className="px-5 py-2.5 text-sm"
              disabled={busy || !status?.clientIdSet}
              onClick={() =>
                void run(async () => {
                  // Se guarda lo que devuelve el backend: si no, un «Guardar» posterior desactivaría las peticiones.
                  setCfg(await api.spotifySetConfig({ ...cfg, song: { ...song, enabled: true } }));
                  await api.spotifyConnect();
                  setMsg(t("spotify.opened"));
                })
              }
            >
              🎵 {t("spotify.connect")}
            </Btn>
            {status && !status.clientIdSet && <p className="text-xs text-amber-300">{t("spotify.unavailable")}</p>}
            {msg && <p className="text-xs text-zinc-400">{msg}</p>}
          </div>
        ) : (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center gap-3">
              <span className="text-sm text-emerald-400">✅ {t("spotify.connected")}</span>
              <Btn variant="danger" disabled={busy} onClick={() => void run(async () => { await api.spotifyDisconnect(); setMsg(null); refreshStatus(); })}>
                {t("spotify.disconnect")}
              </Btn>
            </div>
            <Checkbox
              checked={song.enabled}
              onChange={(enabled) => {
                setSong({ enabled });
                void save({ ...cfg, song: { ...song, enabled } });
              }}
              label={t("spotify.enabledEasy")}
            />
            <p className="text-xs text-zinc-500">{t("spotify.usage")}</p>
          </div>
        )}

        {connected && (
          <details className="rounded-md border border-zinc-800 p-3">
            <summary className="cursor-pointer text-sm text-zinc-300">{t("spotify.options")}</summary>
            <div className="mt-3 space-y-3">
              <div className="grid grid-cols-2 gap-2">
                <Field label={t("spotify.minRole")}>
                  <Select
                    value={song.minRole}
                    onChange={(minRole) => setSong({ minRole })}
                    options={(["everyone", "follower", "subscriber", "moderator"] as const).map((r) => ({ value: r, label: t(`spotify.roles.${r}`) }))}
                  />
                </Field>
                <Field label={t("spotify.cost")} hint={t("spotify.costHint")}>
                  <NumberInput value={song.costPoints} min={0} onChange={(v) => setSong({ costPoints: v ?? 0 })} />
                </Field>
                <Field label={t("spotify.perUser")}>
                  <NumberInput value={song.perUserLimit} min={1} max={50} onChange={(v) => setSong({ perUserLimit: v ?? 1 })} />
                </Field>
                <Field label={t("spotify.cooldown")}>
                  <NumberInput value={song.userCooldownS} min={0} max={3600} onChange={(v) => setSong({ userCooldownS: v ?? 0 })} />
                </Field>
              </div>
              <Checkbox checked={song.reply} onChange={(reply) => setSong({ reply })} label={t("spotify.reply")} />
              <details>
                <summary className="cursor-pointer text-xs text-zinc-400">{t("spotify.moreOptions")}</summary>
                <div className="mt-2 grid grid-cols-2 gap-2">
                  <Field label={t("spotify.commands")} hint={t("spotify.commandsHint")}>
                    <TextInput value={lines(song.commands)} onChange={(v) => setSong({ commands: parseList(v) })} />
                  </Field>
                  <Field label={t("spotify.nowCommands")}>
                    <TextInput value={lines(song.nowPlayingCommands)} onChange={(v) => setSong({ nowPlayingCommands: parseList(v) })} />
                  </Field>
                  <Field label={t("spotify.maxDuration")}>
                    <NumberInput value={song.maxDurationS} min={30} max={10800} onChange={(v) => setSong({ maxDurationS: v ?? 600 })} />
                  </Field>
                  <Field label={t("spotify.blockedTerms")} hint={t("spotify.blockedTermsHint")}>
                    <TextInput value={lines(song.blockedTerms)} onChange={(v) => setSong({ blockedTerms: parseList(v) })} />
                  </Field>
                  <Field label={t("spotify.blockedUsers")}>
                    <TextInput value={lines(song.blockedUsers)} onChange={(v) => setSong({ blockedUsers: parseList(v) })} />
                  </Field>
                </div>
              </details>
              <Btn variant="primary" disabled={busy} onClick={() => void save(cfg).then(() => setMsg(t("integrations.saved")))}>
                {t("integrations.save")}
              </Btn>
            </div>
          </details>
        )}

        {connected && (
          <Field label={t("spotify.test")} hint={t("spotify.testHint")}>
            <div className="flex gap-2">
              <div className="flex-1">
                <TextInput value={query} onChange={setQuery} placeholder="Bohemian Rhapsody" />
              </div>
              <Btn
                disabled={busy || query.trim() === ""}
                onClick={() =>
                  void run(async () => {
                    const tr = await api.spotifyQueueTest(query);
                    setMsg(t("spotify.queued", { title: tr.name, artist: tr.artists.join(", ") }));
                  })
                }
              >
                {t("spotify.queueIt")}
              </Btn>
            </div>
          </Field>
        )}
        {connected && msg && <p className="text-xs text-emerald-400">{msg}</p>}
        <ErrorText error={error} />

        {/* Solo para quien quiera su propia app de Spotify */}
        <details className="rounded-md border border-zinc-800 p-3">
          <summary className="cursor-pointer text-xs text-zinc-500">{t("spotify.advanced")}</summary>
          <div className="mt-3 space-y-2">
            <p className="text-xs text-zinc-500">{t("spotify.advancedHint")}</p>
            <Field label={t("spotify.clientId")} hint={t("spotify.clientIdHint")}>
              <TextInput value={cfg.clientId} onChange={(clientId) => setCfg({ ...cfg, clientId })} autoComplete="off" spellCheck={false} />
            </Field>
            {status && (
              <Field label={t("spotify.redirect")} hint={t("spotify.redirectHint")}>
                <code className="block select-all break-all rounded-md border border-zinc-800 bg-zinc-950 p-2 text-xs text-zinc-300">{status.redirectUri}</code>
              </Field>
            )}
            <Btn disabled={busy} onClick={() => void save(cfg).then(() => setMsg(t("integrations.saved")))}>
              {t("integrations.save")}
            </Btn>
          </div>
        </details>
      </div>
    </Card>
  );
}
