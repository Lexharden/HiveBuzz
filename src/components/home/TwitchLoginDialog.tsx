import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../../lib/api";
import { twitchApi, type DeviceInfo } from "../../lib/twitch";
import { Btn } from "../ui";

type Phase = { kind: "starting" } | { kind: "waiting"; info: DeviceInfo } | { kind: "done" } | { kind: "error"; message: string };

/**
 * Inicio de sesión de Twitch con código en pantalla: el streamer escribe el código en twitch.tv/activate
 * y esta ventana lo detecta sola. No se escribe ninguna contraseña en HiveBuzz.
 */
export function TwitchLoginDialog({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const { t } = useTranslation();
  const [phase, setPhase] = useState<Phase>({ kind: "starting" });
  const [secondsLeft, setSecondsLeft] = useState(0);
  const [copied, setCopied] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const alive = useRef(true);

  const start = useCallback(async () => {
    setPhase({ kind: "starting" });
    try {
      const info = await twitchApi.loginStart();
      if (!alive.current) return;
      setSecondsLeft(info.expiresInS);
      setPhase({ kind: "waiting", info });
      // Se abre la página de activación por comodidad; el código sigue visible por si no se abre.
      void twitchApi.openActivation().catch(() => undefined);
    } catch (e) {
      if (alive.current) setPhase({ kind: "error", message: errorMessage(e) });
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    void start();
    return () => {
      alive.current = false;
      void twitchApi.loginCancel().catch(() => undefined);
    };
  }, [start]);

  // Sondeo del estado y cuenta atrás mientras se espera.
  useEffect(() => {
    if (phase.kind !== "waiting") return;
    const poll = setInterval(() => {
      void twitchApi
        .loginState()
        .then((s) => {
          if (!alive.current) return;
          if (s.state === "done") {
            setPhase({ kind: "done" });
            setTimeout(() => alive.current && onDone(), 1200);
          } else if (s.state === "failed") {
            setPhase({ kind: "error", message: s.reason });
          }
        })
        .catch(() => undefined);
    }, 1500);
    const tick = setInterval(() => setSecondsLeft((s) => Math.max(0, s - 1)), 1000);
    return () => {
      clearInterval(poll);
      clearInterval(tick);
    };
  }, [phase.kind, onDone]);

  useEffect(() => {
    cancelRef.current?.focus();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Sin permiso de portapapeles: el código sigue a la vista.
    }
  };
  const mmss = `${Math.floor(secondsLeft / 60)}:${String(secondsLeft % 60).padStart(2, "0")}`;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div role="dialog" aria-modal="true" aria-labelledby="twitch-login-title" className="w-full max-w-md rounded-2xl border border-[#9146ff]/60 bg-brand-900 p-6 shadow-2xl">
        <h2 id="twitch-login-title" className="text-lg font-bold text-white">
          {t("twitch.login.title")}
        </h2>

        {phase.kind === "starting" && <p className="mt-4 text-sm text-zinc-300">{t("twitch.login.starting")}</p>}

        {phase.kind === "waiting" && (
          <div className="mt-4 space-y-4">
            <ol className="space-y-1 text-sm text-zinc-300">
              <li>1. {t("twitch.login.step1")}</li>
              <li>2. {t("twitch.login.step2")}</li>
              <li>3. {t("twitch.login.step3")}</li>
            </ol>
            <div className="rounded-xl bg-brand-950 p-4 text-center">
              <div className="text-xs text-zinc-400">{t("twitch.login.yourCode")}</div>
              <div aria-live="polite" className="select-all font-mono text-4xl font-bold tracking-[0.3em] text-amber-400">
                {phase.info.userCode}
              </div>
              <div className="mt-3 flex justify-center gap-2">
                <Btn onClick={() => void copy(phase.info.userCode)}>{copied ? t("settings.copied") : t("twitch.login.copy")}</Btn>
                <Btn variant="primary" onClick={() => void twitchApi.openActivation().catch(() => undefined)}>
                  {t("twitch.login.open")}
                </Btn>
              </div>
            </div>
            <p className="flex items-center gap-2 text-xs text-zinc-400">
              <span className="h-2 w-2 animate-pulse rounded-full bg-sky-400" />
              {t("twitch.login.waiting", { time: mmss })}
            </p>
          </div>
        )}

        {phase.kind === "done" && <p className="mt-4 rounded-lg bg-emerald-500/10 p-3 text-sm text-emerald-300">✅ {t("twitch.login.done")}</p>}

        {phase.kind === "error" && (
          <div className="mt-4 space-y-3">
            <p className="rounded-lg bg-rose-500/10 p-3 text-sm text-rose-300">{phase.message}</p>
            <Btn variant="primary" onClick={() => void start()}>
              {t("twitch.login.retry")}
            </Btn>
          </div>
        )}

        <div className="mt-5 flex justify-end">
          <button ref={cancelRef} onClick={onClose} className="rounded-md px-3 py-1.5 text-xs text-zinc-300 hover:bg-brand-800">
            {phase.kind === "done" ? t("twitch.login.close") : t("twitch.login.cancel")}
          </button>
        </div>
        <p className="mt-3 text-[11px] text-zinc-500">{t("twitch.login.privacy")}</p>
      </div>
    </div>
  );
}
