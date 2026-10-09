import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../../lib/api";
import { isActive } from "../../lib/connectionText";
import { twitchApi, type TwitchStatus } from "../../lib/twitch";
import type { StatusUpdate } from "../../lib/types";
import { Btn } from "../ui";
import { TwitchLoginDialog } from "./TwitchLoginDialog";

/**
 * Bajo la tarjeta de Twitch: el chat funciona sin cuenta; entrar con Twitch es una mejora opcional
 * (seguidores, saber si estás en directo y cuántos te ven). Se explica en una frase y se hace con un botón.
 */
export function TwitchUpgrade({ status, channel }: { status: StatusUpdate; channel: string | null }) {
  const { t } = useTranslation();
  const [tw, setTw] = useState<TwitchStatus | null>(null);
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => void twitchApi.status().then(setTw).catch(() => undefined), []);
  useEffect(refresh, [refresh]);

  // Al cambiar de estado de conexión se vuelve a mirar (p. ej. tras conectar se confirma de qué cuenta es la sesión).
  const state = status.state;
  useEffect(refresh, [state, refresh]);

  if (!tw || !tw.loginAvailable) return null;

  const done = async () => {
    setOpen(false);
    refresh();
    // La sesión nueva se aplica al volver a entrar al canal.
    if (isActive(status) && channel) {
      try {
        await api.connect("twitch", channel);
      } catch (e) {
        setError(errorMessage(e));
      }
    }
  };

  return (
    <div className="mt-3 border-t border-brand-700/60 pt-3">
      {!tw.loggedIn ? (
        <div className="rounded-lg bg-[#9146ff]/10 p-3">
          <p className="text-sm text-zinc-200">{t("twitch.upgrade.pitch")}</p>
          <Btn variant="primary" className="mt-2" onClick={() => setOpen(true)}>
            {t("twitch.upgrade.button")}
          </Btn>
        </div>
      ) : (
        <div className="space-y-1">
          <div className="flex items-center justify-between gap-2">
            <p className="text-sm text-emerald-300">✓ {tw.account ? t("twitch.upgrade.loggedAs", { login: tw.account.login }) : t("twitch.upgrade.logged")}</p>
            <Btn
              onClick={() =>
                void twitchApi
                  .logout()
                  .then(refresh)
                  .catch((e: unknown) => setError(errorMessage(e)))
              }
            >
              {t("twitch.upgrade.logout")}
            </Btn>
          </div>
          {tw.account && channel && tw.account.login !== channel.toLowerCase().replace(/^[#@]/, "") && <p className="text-xs text-amber-300">{t("twitch.upgrade.otherChannel", { login: tw.account.login })}</p>}
          <p className="text-xs text-zinc-500">{t("twitch.upgrade.benefits")}</p>
        </div>
      )}
      {error && (
        <p role="alert" className="mt-2 text-xs text-rose-300">
          {error}
        </p>
      )}
      {open && <TwitchLoginDialog onClose={() => setOpen(false)} onDone={() => void done()} />}
    </div>
  );
}
