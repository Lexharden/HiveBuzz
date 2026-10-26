import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../../lib/api";
import type { AppInfo, OverlayConfig, OverlayDef } from "../../lib/types";
import { Btn, Card, ErrorText, toast } from "../ui";
import { Preview } from "./Preview";
import { SchemaForm } from "./SchemaForm";

/** Overlays que tienen botón «probar» (los demás muestran datos reales). */
const TESTABLE = new Set(["chat", "feed", "gifts", "nowplaying", "songqueue", "jar"]);
/** Las pantallas de alertas (`alerts`, `alerts2`…) también se pueden probar. */
const isTestable = (d: OverlayDef) => TESTABLE.has(d.id) || d.screen !== undefined;
/** Cuánto se espera antes de enviar un cambio (mientras se arrastra un color o se teclea). */
const SEND_DELAY_MS = 120;

export function OverlaysPage({ info }: { info: AppInfo | null }) {
  const { t } = useTranslation();
  const [defs, setDefs] = useState<OverlayDef[]>([]);
  const [selected, setSelected] = useState<string>("alerts");
  /** Pantallas de alertas creadas (las demás pantallas del esquema no se muestran). */
  const [screens, setScreens] = useState(5);
  const [config, setConfig] = useState<OverlayConfig>({});
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const alive = useRef(true);

  useEffect(() => {
    alive.current = true;
    const pending = timers.current;
    void api.listOverlays().then((d) => alive.current && setDefs(d)).catch((e: unknown) => setError(errorMessage(e)));
    void api.getAlertScreens().then((n) => alive.current && setScreens(n)).catch(() => undefined);
    return () => {
      alive.current = false;
      pending.forEach(clearTimeout);
    };
  }, []);

  useEffect(() => {
    void api.getOverlayConfig(selected).then((c) => alive.current && setConfig(c)).catch((e: unknown) => setError(errorMessage(e)));
  }, [selected]);

  const visible = defs.filter((d) => d.screen === undefined || d.screen <= screens);
  const def = visible.find((d) => d.id === selected);
  const nameOf = (d: OverlayDef) => t(d.name, { n: d.screen });

  /** Crea (o quita) pantallas de alertas; si se quita la que se está viendo, vuelve a la primera. */
  async function resizeScreens(count: number) {
    try {
      const n = await api.setAlertScreens(count);
      setScreens(n);
      const last = defs.find((d) => d.screen === n);
      if (count > screens && last) setSelected(last.id);
      else if (!defs.some((d) => d.id === selected && (d.screen === undefined || d.screen <= n))) setSelected("alerts");
    } catch (e) {
      setError(errorMessage(e));
    }
  }
  const url = info?.overlays.find((o) => o.id === selected)?.url;

  /** Aplica el cambio en pantalla al instante y lo manda al backend con un pequeño retardo. */
  const change = useCallback(
    (key: string, value: string | number | boolean) => {
      setConfig((c) => ({ ...c, [key]: value }));
      const pending = timers.current;
      const prev = pending.get(key);
      if (prev) clearTimeout(prev);
      pending.set(
        key,
        setTimeout(() => {
          pending.delete(key);
          setError(null);
          api
            .setOverlayConfig(selected, { [key]: value })
            // El backend devuelve el valor ya validado (acotado al rango, ajustado al paso).
            .then((merged) => alive.current && setConfig(merged))
            .catch((e: unknown) => {
              setError(errorMessage(e));
              void api.getOverlayConfig(selected).then((c) => alive.current && setConfig(c)).catch(() => undefined);
            });
        }, SEND_DELAY_MS),
      );
    },
    [selected],
  );

  async function copyUrl(text: string | undefined = url) {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      setError(t("overlays.copyFailed"));
    }
  }

  return (
    <div className="flex min-h-0 flex-1 gap-4">
      <nav className="w-44 shrink-0 space-y-1 overflow-y-auto">
        {visible.map((d) => (
          <button
            key={d.id}
            onClick={() => setSelected(d.id)}
            className={`w-full rounded-md px-3 py-2 text-left text-sm ${selected === d.id ? "bg-zinc-700 font-semibold" : "text-zinc-400 hover:bg-zinc-900 hover:text-zinc-200"}`}
          >
            {nameOf(d)}
          </button>
        ))}
        <div className="space-y-1 border-t border-zinc-800 pt-2">
          <Btn onClick={() => void resizeScreens(screens + 1)} disabled={defs.every((d) => d.screen === undefined || d.screen <= screens)}>
            {t("overlays.addScreen")}
          </Btn>
          <Btn
            onClick={() => {
              if (window.confirm(t("overlays.removeScreenConfirm", { n: screens }))) void resizeScreens(screens - 1);
            }}
            disabled={screens <= 1}
          >
            {t("overlays.removeScreen")}
          </Btn>
        </div>
      </nav>

      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto pr-1">
        {def && (
          <>
            <Card
              title={nameOf(def)}
              hint={t("overlays.urlHint")}
              actions={
                <div className="flex gap-2">
                  <Btn onClick={() => void copyUrl()} disabled={!url}>{copied ? t("settings.copied") : t("settings.copy")}</Btn>
                  <Btn onClick={() => void copyUrl(url?.replace("//127.0.0.1:", "//localhost:"))} disabled={!url} title={t("overlays.copyLocalhostHint")}>
                    {t("overlays.copyLocalhost")}
                  </Btn>
                  {isTestable(def) && <Btn onClick={() => api.testOverlay(def.id).then(() => toast(t("overlays.testSent"))).catch((e: unknown) => setError(errorMessage(e)))}>▶ {t("overlays.test")}</Btn>}
                  <Btn
                    onClick={() => {
                      if (window.confirm(t("overlays.confirmReset"))) void api.resetOverlayConfig(def.id).then(setConfig).catch((e: unknown) => setError(errorMessage(e)));
                    }}
                  >
                    {t("overlays.reset")}
                  </Btn>
                </div>
              }
            >
              {url ? <Preview url={url} /> : <p className="text-sm text-zinc-500">{t("overlays.noServer")}</p>}
              <p className="mt-2 text-xs text-zinc-500">{isTestable(def) ? t("overlays.previewHintTest") : t("overlays.previewHintLive")}</p>
            </Card>

            {def.id === "jar" && (
              <JarControls target={Number(config.target) || 0} onError={(e) => setError(errorMessage(e))} />
            )}

            <Card>
              <SchemaForm fields={def.fields} config={config} onChange={change} />
            </Card>
            <ErrorText error={error} />
          </>
        )}
      </div>
    </div>
  );
}

/** Acciones del jarrón: ver cuánto lleva, sumar a mano, llenarlo o vaciarlo. */
function JarControls({ target, onError }: { target: number; onError: (e: unknown) => void }) {
  const { t } = useTranslation();
  const [state, setState] = useState({ coins: 0, laps: 0 });

  useEffect(() => {
    let alive = true;
    const refresh = () => void api.jarState().then((s) => alive && setState(s)).catch(() => undefined);
    refresh();
    const id = setInterval(refresh, 1000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  const run = (op: Promise<void>) => void op.catch(onError);

  return (
    <Card title={t("overlays.jarCoins")} hint={`${state.coins.toLocaleString()} / ${target.toLocaleString()} · ${t("overlays.jarLaps")}: ${state.laps}`}>
      <div className="flex flex-wrap gap-2">
        {[100, 1000, 10000].map((n) => (
          <Btn key={n} onClick={() => run(api.jarAdd(n))}>+{n.toLocaleString()}</Btn>
        ))}
        <Btn onClick={() => run(api.jarSet(target))} disabled={target <= 0}>{t("overlays.jarFill")}</Btn>
        <Btn
          onClick={() => {
            if (window.confirm(t("overlays.jarConfirmEmpty"))) run(api.jarReset());
          }}
        >
          {t("overlays.jarEmpty")}
        </Btn>
      </div>
    </Card>
  );
}
