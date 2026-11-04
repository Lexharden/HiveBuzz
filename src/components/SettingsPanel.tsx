import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../lib/api";
import type { AppInfo, HttpsStatus } from "../lib/types";

function CopyButton({ text }: { text: string }) {
  const { t } = useTranslation();
  const [done, setDone] = useState(false);
  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      setDone(true);
      setTimeout(() => setDone(false), 1500);
    } catch {
      // Sin permiso de portapapeles: el usuario aún puede seleccionar la URL a mano.
    }
  }
  return (
    <button onClick={() => void copy()} className="rounded-md bg-zinc-800 px-3 py-1.5 text-xs hover:bg-zinc-700">
      {done ? t("settings.copied") : t("settings.copy")}
    </button>
  );
}

/** Número de pantalla de un overlay de alertas (`alerts` = 1, `alerts2` = 2…); sin número para los demás. */
function screenOf(id: string): number | undefined {
  const m = /^alerts(\d*)$/.exec(id);
  return m ? Number(m[1] || 1) : undefined;
}

/** HTTPS local: TikTok LIVE Studio no acepta URLs `http://`. Requiere instalar el certificado de HiveBuzz una vez. */
function HttpsSection({ info }: { info: AppInfo }) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<HttpsStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function refresh() {
    try {
      setStatus(await api.httpsStatus());
    } catch (e) {
      setError(errorMessage(e));
    }
  }
  useEffect(() => {
    void refresh();
  }, []);

  async function install() {
    setBusy(true);
    setError(null);
    try {
      await api.installLocalCa();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
      await refresh();
    }
  }

  return (
    <section className="rounded-xl border border-brand-700/70 bg-brand-900 p-4">
      <h2 className="text-sm font-semibold text-zinc-300">{t("settings.https")}</h2>
      <p className="mt-1 text-xs text-zinc-500">{t("settings.httpsHint")}</p>
      {info.httpsError ? (
        <p className="mt-2 text-xs text-rose-300">{t("settings.httpsError")}: {info.httpsError}</p>
      ) : (
        <>
          <p className="mt-2 text-xs text-zinc-400">
            {status?.installed === true && <span className="text-emerald-400">{t("settings.httpsInstalled")}</span>}
            {status?.installed === false && <span className="text-amber-300">{t("settings.httpsNotInstalled")}</span>}
            {status?.installed === null && <span>{t("settings.httpsManual", { path: status.caPath ?? "" })}</span>}
          </p>
          {status?.installed === false && (
            <div className="mt-2">
              <button disabled={busy} onClick={() => void install()} className="rounded-md bg-amber-400 px-3 py-1.5 text-xs font-semibold text-zinc-950 disabled:opacity-50">
                {t("settings.httpsInstall")}
              </button>
              <p className="mt-1 text-xs text-zinc-500">{t("settings.httpsInstallHint")}</p>
            </div>
          )}
          <p className="mt-2 text-xs text-zinc-500">{t("settings.httpsWhere")}</p>
        </>
      )}
      {error && <p className="mt-2 text-xs text-rose-300">{error}</p>}
    </section>
  );
}

export function SettingsPanel({ info, onChanged }: { info: AppInfo; onChanged: () => Promise<void> }) {
  const { t } = useTranslation();
  const [port, setPort] = useState(String(info.serverPort));
  const [error, setError] = useState<string | null>(null);

  async function run(action: () => Promise<void>) {
    setError(null);
    try {
      await action();
      await onChanged();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <div className="space-y-4">
      {info.serverError && (
        <p className="rounded-lg border border-rose-500/40 bg-rose-500/10 p-3 text-sm text-rose-300">
          {t("settings.serverError")}: {info.serverError}
        </p>
      )}

      <section className="rounded-xl border border-brand-700/70 bg-brand-900 p-4">
        <h2 className="text-sm font-semibold text-zinc-300">{t("settings.overlays")}</h2>
        <p className="mt-1 text-xs text-zinc-500">{t("settings.overlaysHint")}</p>
        <ul className="mt-3 space-y-2">
          {info.overlays.map((o) => (
            <li key={o.id} className="flex items-center gap-2">
              <span className="w-40 shrink-0 text-sm">{t(o.name, { n: screenOf(o.id) })}</span>
              <input readOnly value={o.url} onFocus={(e) => e.currentTarget.select()}
                className="min-w-0 flex-1 rounded-md border border-zinc-700 bg-zinc-950 px-2 py-1.5 font-mono text-xs" />
              <CopyButton text={o.url} />
            </li>
          ))}
        </ul>
      </section>

      <HttpsSection info={info} />

      <section className="rounded-xl border border-brand-700/70 bg-brand-900 p-4">
        <h2 className="text-sm font-semibold text-zinc-300">{t("settings.port")}</h2>
        <p className="mt-1 text-xs text-zinc-500">{t("settings.portHint")}</p>
        <div className="mt-3 flex gap-2">
          <input
            type="number"
            min={1024}
            max={65535}
            value={port}
            onChange={(e) => setPort(e.target.value)}
            className="w-32 rounded-md border border-zinc-700 bg-zinc-950 px-2 py-1.5 text-sm"
          />
          <button
            disabled={Number(port) === info.serverPort}
            onClick={() => run(() => api.setServerPort(Number(port)))}
            className="rounded-md bg-amber-400 px-3 py-1.5 text-xs font-semibold text-zinc-950 disabled:opacity-50"
          >
            {t("settings.save")}
          </button>
        </div>
      </section>

      {error && <p className="text-sm text-rose-400">{error}</p>}
    </div>
  );
}
