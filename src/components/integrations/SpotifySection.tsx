import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { systemApi as api, type SpotifyConfig, type SpotifyOp, type SpotifyOverview, type SpotifyStatus } from "../../lib/system";
import { Btn, Card, Checkbox, ErrorText, Field, NumberInput, Select, TextInput, useAction } from "../ui";

const DASHBOARD_URL = "https://developer.spotify.com/dashboard";

const lines = (v: string[]): string => v.join(", ");
const parseList = (raw: string): string[] =>
  raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);

/** Reproductor: botones de control, lo que suena y las próximas canciones. */
function PlayerPanel({ onError }: { onError: (e: unknown) => void }) {
  const { t } = useTranslation();
  const [view, setView] = useState<SpotifyOverview | null>(null);
  const refresh = useCallback(() => void api.spotifyOverview().then(setView).catch(() => undefined), []);
  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, 2000);
    return () => clearInterval(timer);
  }, [refresh]);

  const send = (op: SpotifyOp, value?: number) => {
    void api
      .spotifyControl(op, value)
      .catch(onError)
      // Spotify tarda un instante en reflejar el cambio.
      .finally(() => setTimeout(refresh, 700));
  };
  const playing = view?.playing ?? null;
  return (
    <div className="space-y-2 rounded-md border border-zinc-800 p-3">
      <p className="text-sm text-zinc-200">
        {playing ? `${playing.isPlaying ? "▶" : "⏸"} ${playing.track.name} — ${playing.track.artists.join(", ")}` : t("spotify.nothingPlaying")}
      </p>
      <div className="flex flex-wrap gap-2">
        <Btn onClick={() => send("previous")} title={t("spotify.prev")}>⏮</Btn>
        <Btn onClick={() => send("pause")} title={t("spotify.pause")}>⏸</Btn>
        <Btn onClick={() => send("resume")} title={t("spotify.resume")}>▶</Btn>
        <Btn onClick={() => send("next")} title={t("spotify.next")}>⏭</Btn>
        <Btn onClick={() => send("volumeChange", -10)} title={t("spotify.volDown")}>🔉 −</Btn>
        <Btn onClick={() => send("volumeChange", 10)} title={t("spotify.volUp")}>🔊 +</Btn>
      </div>
      <p className="text-xs text-zinc-500">{t("spotify.queueTitle")}</p>
      {view && view.queue.length > 0 ? (
        <ol className="list-decimal space-y-0.5 pl-5 text-xs text-zinc-300">
          {view.queue.slice(0, 8).map((q, i) => (
            <li key={`${q.track.uri}-${i}`}>
              {q.track.name} — {q.track.artists.join(", ")}
              {q.requestedBy && <span className="text-amber-400"> · @{q.requestedBy}</span>}
            </li>
          ))}
        </ol>
      ) : (
        <p className="text-xs text-zinc-500">{t("spotify.queueEmpty")}</p>
      )}
    </div>
  );
}

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
  const setControl = (patch: Partial<typeof cfg.control>) => setCfg({ ...cfg, control: { ...cfg.control, ...patch } });
  const save = (next: SpotifyConfig) =>
    run(async () => {
      setCfg(await api.spotifySetConfig(next));
      refreshStatus();
    });
  const connected = status?.connected ?? false;
  // Sin app propia (ni integrada) no se puede conectar: la guía se muestra abierta.
  const needsSetup = status !== null && !status.clientIdSet;

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
            {needsSetup && <p className="text-xs text-amber-300">{t("spotify.needsSetup")}</p>}
            {status?.usesBuiltinApp && <p className="text-xs text-zinc-400">{t("spotify.builtinNote")}</p>}
            {msg && <p className="text-xs text-zinc-400">{msg}</p>}
          </div>
        ) : (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center gap-3">
              <span className="text-sm text-emerald-400">✅ {t("spotify.connected")}</span>
              <Btn variant="danger" disabled={busy} onClick={() => run(async () => { await api.spotifyDisconnect(); setMsg(null); refreshStatus(); })}>
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

        {connected && <PlayerPanel onError={(e) => setMsg(String(e))} />}

        {connected && (
          <details className="rounded-md border border-zinc-800 p-3">
            <summary className="cursor-pointer text-sm text-zinc-300">{t("spotify.controlTitle")}</summary>
            <div className="mt-3 space-y-3">
              <Checkbox checked={cfg.control.enabled} onChange={(enabled) => setControl({ enabled })} label={t("spotify.controlEnabled")} />
              <p className="text-xs text-zinc-500">{t("spotify.controlHint")}</p>
              <div className="grid grid-cols-2 gap-2">
                <Field label={t("spotify.controlRole")}>
                  <Select
                    value={cfg.control.minRole}
                    onChange={(minRole) => setControl({ minRole })}
                    options={(["everyone", "follower", "subscriber", "moderator"] as const).map((r) => ({ value: r, label: t(`spotify.roles.${r}`) }))}
                  />
                </Field>
                <Field label={t("spotify.skipVotes")} hint={t("spotify.skipVotesHint")}>
                  <NumberInput value={cfg.control.skipVotes} min={0} max={1000} onChange={(v) => setControl({ skipVotes: v ?? 0 })} />
                </Field>
                <Field label={t("spotify.volumeStep")}>
                  <NumberInput value={cfg.control.volumeStep} min={1} max={50} onChange={(v) => setControl({ volumeStep: v ?? 10 })} />
                </Field>
                <Field label={t("spotify.queueShowS")}>
                  <NumberInput value={cfg.control.queueShowS} min={3} max={300} onChange={(v) => setControl({ queueShowS: v ?? 15 })} />
                </Field>
                <Field label={t("spotify.cmdSkip")}>
                  <TextInput value={lines(cfg.control.skipCommands)} onChange={(v) => setControl({ skipCommands: parseList(v) })} />
                </Field>
                <Field label={t("spotify.cmdPause")}>
                  <TextInput value={lines(cfg.control.pauseCommands)} onChange={(v) => setControl({ pauseCommands: parseList(v) })} />
                </Field>
                <Field label={t("spotify.cmdResume")}>
                  <TextInput value={lines(cfg.control.resumeCommands)} onChange={(v) => setControl({ resumeCommands: parseList(v) })} />
                </Field>
                <Field label={t("spotify.cmdVolume")}>
                  <TextInput value={lines(cfg.control.volumeCommands)} onChange={(v) => setControl({ volumeCommands: parseList(v) })} />
                </Field>
                <Field label={t("spotify.cmdQueue")} hint={t("spotify.cmdQueueHint")}>
                  <TextInput value={lines(cfg.control.queueCommands)} onChange={(v) => setControl({ queueCommands: parseList(v) })} />
                </Field>
              </div>
              <Btn variant="primary" disabled={busy} onClick={() => save(cfg).then(() => setMsg(t("integrations.saved")))}>
                {t("integrations.save")}
              </Btn>
            </div>
          </details>
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
              <Checkbox checked={song.noRepeat} onChange={(noRepeat) => setSong({ noRepeat })} label={t("spotify.noRepeat")} />
              <details>
                <summary className="cursor-pointer text-xs text-zinc-400">{t("spotify.moreOptions")}</summary>
                <div className="mt-2 grid grid-cols-2 gap-2">
                  <Field label={t("spotify.commands")} hint={t("spotify.commandsHint")}>
                    <TextInput value={lines(song.commands)} onChange={(v) => setSong({ commands: parseList(v) })} />
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
              <Btn variant="primary" disabled={busy} onClick={() => save(cfg).then(() => setMsg(t("integrations.saved")))}>
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

        {/* La app de Spotify de cada streamer: Spotify solo deja usar una app en modo desarrollo a
            las cuentas que su dueño añade a mano, así que lo fiable es que cada uno tenga la suya. */}
        <details className="rounded-md border border-zinc-800 p-3" open={needsSetup}>
          <summary className="cursor-pointer text-sm text-zinc-300">{t("spotify.setupTitle")}</summary>
          <div className="mt-3 space-y-3">
            <p className="text-xs text-zinc-400">{t("spotify.setupIntro")}</p>
            <ol className="list-decimal space-y-1.5 pl-5 text-xs text-zinc-300">
              <li>
                {t("spotify.step1")}{" "}
                <button type="button" className="text-amber-400 underline" onClick={() => void openUrl(DASHBOARD_URL)}>
                  developer.spotify.com/dashboard
                </button>
              </li>
              <li>{t("spotify.step2")}</li>
              <li>{t("spotify.step3")}</li>
              <li>{t("spotify.step4")}</li>
              <li>{t("spotify.step5")}</li>
            </ol>
            <p className="text-xs text-zinc-500">{t("spotify.forbiddenHelp")}</p>
            <Field label={t("spotify.clientId")} hint={t("spotify.clientIdHint")}>
              <TextInput value={cfg.clientId} onChange={(clientId) => setCfg({ ...cfg, clientId })} autoComplete="off" spellCheck={false} />
            </Field>
            {status && (
              <Field label={t("spotify.redirect")} hint={t("spotify.redirectHint")}>
                <code className="block select-all break-all rounded-md border border-zinc-800 bg-zinc-950 p-2 text-xs text-zinc-300">{status.redirectUri}</code>
              </Field>
            )}
            <Btn disabled={busy} onClick={() => save(cfg).then(() => setMsg(t("integrations.saved")))}>
              {t("integrations.save")}
            </Btn>
          </div>
        </details>
      </div>
    </Card>
  );
}
