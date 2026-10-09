import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../../lib/api";
import { isActive, statusKey } from "../../lib/connectionText";
import type { Platform, StatusUpdate } from "../../lib/types";
import { PlatformBadge } from "../PlatformBadge";

const DOT: Record<StatusUpdate["state"], string> = {
  disconnected: "bg-zinc-500",
  waiting_live: "bg-sky-400",
  connected: "bg-emerald-400",
  signature_error: "bg-rose-500",
  reconnecting: "bg-amber-400",
};

const ACCENT: Record<Platform, string> = { tiktok: "border-l-[#fe2c55]", twitch: "border-l-[#9146ff]" };

interface Props {
  platform: Platform;
  status: StatusUpdate;
  /** Último nombre usado, para no tener que escribirlo cada vez. */
  lastName: string | null;
  /** Contenido extra bajo el formulario (p. ej. el inicio de sesión de Twitch). */
  children?: ReactNode;
  /** Se llama tras conectar o desconectar con éxito. */
  onChanged?: () => void;
}

/** Una plataforma: su estado en palabras, un campo y un botón. Nada más a la vista. */
export function ConnectionCard({ platform, status, lastName, children, onChanged }: Props) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const active = isActive(status);
  const prefix = platform === "tiktok" ? "@" : "";

  useEffect(() => {
    if (lastName) setName((prev) => prev || `${prefix}${lastName}`);
  }, [lastName, prefix]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      if (active) await api.disconnect(platform);
      else await api.connect(platform, name);
      onChanged?.();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  const retry =
    status.attempt !== null && status.retryInMs !== null ? t("status.retryIn", { n: status.attempt, s: Math.ceil(status.retryInMs / 1000) }) : null;
  const pulsing = status.state === "waiting_live" || status.state === "reconnecting";
  const inputId = `conn-${platform}`;

  return (
    <section className={`rounded-xl border border-l-4 border-brand-700/70 bg-brand-900 p-4 ${ACCENT[platform]}`} aria-label={t(`platform.${platform}`)}>
      <div className="flex items-center justify-between gap-2">
        <PlatformBadge platform={platform} className="text-xs" />
        <div className="flex items-center gap-2" role="status" aria-live="polite">
          <span className={`h-2.5 w-2.5 rounded-full ${DOT[status.state]} ${pulsing ? "animate-pulse" : ""}`} aria-hidden />
          <span className="text-sm font-semibold">{t(statusKey(platform, status))}</span>
        </div>
      </div>
      {retry && <p className="mt-1 text-right text-xs text-zinc-400">{retry}</p>}

      <form onSubmit={submit} className="mt-3">
        <label htmlFor={inputId} className="mb-1 block text-xs text-zinc-400">
          {t(`conn.${platform}.label`)}
        </label>
        <div className="flex gap-2">
          <input
            id={inputId}
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t(`conn.${platform}.placeholder`)}
            disabled={active || busy}
            spellCheck={false}
            autoComplete="off"
            className="min-w-0 flex-1 rounded-lg border border-zinc-700 bg-brand-950 px-3 py-2 text-sm outline-none focus:border-amber-400 disabled:opacity-60"
          />
          <button
            type="submit"
            disabled={busy || (!active && name.trim() === "")}
            className={`flex-none rounded-lg px-4 py-2 text-sm font-semibold disabled:opacity-50 ${active ? "bg-brand-700 hover:bg-brand-600" : "bg-amber-400 text-brand-900 hover:bg-amber-300"}`}
          >
            {busy ? t("connect.busy") : active ? t("connect.disconnect") : t("connect.connect")}
          </button>
        </div>
        <p className="mt-1 text-xs text-zinc-500">{t(`conn.${platform}.help`)}</p>
      </form>

      {error && (
        <p role="alert" className="mt-2 rounded-md bg-rose-500/10 p-2 text-xs text-rose-300">
          {error}
        </p>
      )}
      {children}
    </section>
  );
}
