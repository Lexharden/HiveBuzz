import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { integrationsApi as api, type ObsConfig } from "../../lib/integrations";
import type { AppInfo } from "../../lib/types";
import { Btn, Card, ErrorText, Field, NumberInput, TextInput, useAction } from "../ui";
import { SpotifySection } from "./SpotifySection";
import { TwitchSection } from "./TwitchSection";

export function IntegrationsPage({ info }: { info: AppInfo | null }) {
  return (
    <div className="min-h-0 flex-1 space-y-4 overflow-y-auto pb-4">
      <ObsSection />
      <SpotifySection />
      <TwitchSection />
      <ApiSection info={info} />
    </div>
  );
}

function ObsSection() {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<ObsConfig | null>(null);
  const [hasPassword, setHasPassword] = useState(false);
  const [password, setPassword] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const { run, error, busy } = useAction();

  useEffect(() => {
    void api.getObsConfig().then(setCfg);
    void api.hasObsPassword().then(setHasPassword);
  }, []);
  if (!cfg) return null;

  return (
    <Card title={t("integrations.obs.title")} hint={t("integrations.obs.hint")}>
      <div className="space-y-3">
        <div className="grid grid-cols-3 gap-2">
          <Field label={t("integrations.obs.host")}>
            <TextInput value={cfg.host} onChange={(host) => setCfg({ ...cfg, host })} />
          </Field>
          <Field label={t("integrations.obs.port")}>
            <NumberInput value={cfg.port} min={1} max={65535} onChange={(p) => setCfg({ ...cfg, port: p ?? 4455 })} />
          </Field>
          <Field label={t("integrations.obs.password")} hint={hasPassword ? t("integrations.obs.passwordSaved") : t("integrations.obs.passwordNone")}>
            <TextInput type="password" autoComplete="off" value={password} onChange={setPassword} placeholder={hasPassword ? "••••••••" : ""} />
          </Field>
        </div>
        <div className="flex flex-wrap gap-2">
          <Btn
            variant="primary"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                setCfg(await api.setObsConfig(cfg));
                if (password !== "") {
                  await api.setObsPassword(password);
                  setPassword("");
                  setHasPassword(true);
                }
                setMsg(t("integrations.saved"));
              })
            }
          >
            {t("integrations.save")}
          </Btn>
          <Btn
            disabled={busy}
            onClick={() =>
              void run(async () => {
                const v = await api.testObs();
                setMsg(t("integrations.obs.ok", { obs: v.obsVersion, ws: v.websocketVersion }));
              })
            }
          >
            {t("integrations.obs.test")}
          </Btn>
          {hasPassword && (
            <Btn
              variant="danger"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  await api.setObsPassword("");
                  setHasPassword(false);
                  setMsg(null);
                })
              }
            >
              {t("integrations.obs.clearPassword")}
            </Btn>
          )}
        </div>
        {msg && <p className="text-xs text-emerald-400">{msg}</p>}
        <ErrorText error={error} />
        <p className="text-xs text-zinc-500">{t("integrations.obs.help")}</p>
      </div>
    </Card>
  );
}

function ApiSection({ info }: { info: AppInfo | null }) {
  const { t } = useTranslation();
  if (!info) return null;
  const base = `http://127.0.0.1:${info.serverPort}`;
  const curl = `curl -X POST "${base}/api/trigger" -H "Authorization: Bearer ${info.overlayToken}" -H "Content-Type: application/json" -d "{\\"name\\":\\"mi-accion\\",\\"vars\\":{\\"user\\":\\"ana\\"}}"`;
  const ws = `ws://127.0.0.1:${info.serverPort}/ws?token=${info.overlayToken}`;
  return (
    <Card title={t("integrations.api.title")} hint={t("integrations.api.hint")}>
      <div className="space-y-3 text-xs">
        <Field label={t("integrations.api.trigger")} hint={t("integrations.api.triggerHint")}>
          <code className="block select-all break-all rounded-md border border-zinc-800 bg-zinc-950 p-2 text-zinc-300">{curl}</code>
        </Field>
        <Field label={t("integrations.api.ws")} hint={t("integrations.api.wsHint")}>
          <code className="block select-all break-all rounded-md border border-zinc-800 bg-zinc-950 p-2 text-zinc-300">{ws}</code>
        </Field>
        <p className="text-zinc-500">{t("integrations.api.security")}</p>
      </div>
    </Card>
  );
}
