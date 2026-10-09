import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { twitchApi, type TwitchStatus } from "../../lib/twitch";
import { Btn, Card, ErrorText, Field, TextInput, useAction } from "../ui";

/** Solo para quien quiera usar su propia app de Twitch. Plegado: el resto de personas no necesita abrirlo. */
export function TwitchSection() {
  const { t } = useTranslation();
  const [clientId, setClientId] = useState("");
  const [status, setStatus] = useState<TwitchStatus | null>(null);
  const [saved, setSaved] = useState(false);
  const { run, error, busy } = useAction();

  useEffect(() => {
    void twitchApi.getConfig().then((c) => setClientId(c.clientId));
    void twitchApi.status().then(setStatus).catch(() => undefined);
  }, []);

  return (
    <Card title={t("twitch.advanced.title")} hint={t("twitch.advanced.hint")}>
      <details>
        <summary className="cursor-pointer text-xs text-zinc-400">{t("spotify.advanced")}</summary>
        <div className="mt-3 space-y-2">
          {status && !status.loginAvailable && <p className="text-xs text-amber-300">{t("twitch.advanced.unavailable")}</p>}
          <Field label={t("twitch.advanced.clientId")} hint={t("twitch.advanced.clientIdHint")}>
            <TextInput value={clientId} onChange={setClientId} autoComplete="off" spellCheck={false} />
          </Field>
          <Btn
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await twitchApi.setConfig({ clientId });
                setStatus(await twitchApi.status());
                setSaved(true);
              })
            }
          >
            {t("integrations.save")}
          </Btn>
          {saved && !error && <p className="text-xs text-emerald-400">{t("integrations.saved")}</p>}
          <ErrorText error={error} />
        </div>
      </details>
    </Card>
  );
}
