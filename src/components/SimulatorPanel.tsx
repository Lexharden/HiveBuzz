import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../lib/api";
import { setFlag } from "../lib/flags";
import type { Platform, SimKind } from "../lib/types";

const KINDS: Record<Platform, SimKind[]> = {
  tiktok: ["gift", "bigGift", "chat", "like", "follow", "share", "subscribe", "join"],
  // Twitch no tiene likes, shares ni entradas: no se ofrecen botones que no existen allí.
  twitch: ["gift", "bigGift", "chat", "follow", "subscribe"],
};

/** Eventos de mentira para probar sin estar en directo. Plegado: no estorba a quien ya está en vivo. */
export function SimulatorPanel() {
  const { t } = useTranslation();
  const [platform, setPlatform] = useState<Platform>("tiktok");
  const [error, setError] = useState<string | null>(null);
  const [sent, setSent] = useState(false);

  async function run(action: () => Promise<unknown>) {
    setError(null);
    try {
      await action();
      setSent(true);
      setFlag("gs.tested", true);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <details data-tour="simulator" className="rounded-xl border border-brand-700/70 bg-brand-900">
      <summary className="cursor-pointer select-none px-4 py-3 text-sm font-semibold text-zinc-300">🧪 {t("simulator.title")}</summary>
      <div className="px-4 pb-4">
        <p className="text-xs text-zinc-400">{t("simulator.hint")}</p>
        <div className="mt-3 flex items-center gap-2" role="group" aria-label={t("simulator.platform")}>
          <span className="text-xs text-zinc-400">{t("simulator.platform")}</span>
          {(["tiktok", "twitch"] as const).map((p) => (
            <button
              key={p}
              onClick={() => setPlatform(p)}
              aria-pressed={platform === p}
              className={`rounded-full px-3 py-1 text-xs ${platform === p ? "bg-amber-400 font-semibold text-brand-900" : "bg-brand-800 text-zinc-300 hover:bg-brand-700"}`}
            >
              {t(`platform.${p}`)}
            </button>
          ))}
        </div>
        <div className="mt-3 flex flex-wrap gap-2">
          {KINDS[platform].map((k) => (
            <button key={k} onClick={() => void run(() => api.simulateEvent(k, platform))} className="rounded-md bg-brand-700 px-3 py-1.5 text-xs hover:bg-brand-600">
              {t(platform === "twitch" && (k === "gift" || k === "bigGift") ? `simulator.${k}Twitch` : `simulator.${k}`)}
            </button>
          ))}
          <button onClick={() => void run(() => api.simulateBurst(20, platform))} className="rounded-md bg-amber-400 px-3 py-1.5 text-xs font-semibold text-brand-900 hover:bg-amber-300">
            {t("simulator.burst")}
          </button>
        </div>
        {sent && !error && <p className="mt-2 text-xs text-emerald-400">{t("simulator.sent")}</p>}
        {error && (
          <p role="alert" className="mt-2 text-xs text-rose-400">
            {error}
          </p>
        )}
      </div>
    </details>
  );
}
