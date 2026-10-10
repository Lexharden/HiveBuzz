import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../lib/api";
import type { MicGuard, MicGuardMode, MicStatus } from "../../lib/types";
import { Card, Checkbox, Field, NumberInput, Select } from "../ui";

const MIN_DB = -80;
const MAX_DB = 0;
const pct = (db: number) => Math.min(100, Math.max(0, ((db - MIN_DB) / (MAX_DB - MIN_DB)) * 100));

/** «No hablar encima»: el micrófono pausa o salta la lectura mientras el streamer habla. */
export function MicGuardCard({ value, onChange, saved }: { value: MicGuard; onChange: (p: Partial<MicGuard>) => void; saved: MicGuard }) {
  const { t } = useTranslation();
  const [devices, setDevices] = useState<string[]>([]);
  const [status, setStatus] = useState<MicStatus | null>(null);

  useEffect(() => {
    let alive = true;
    void api.listMicDevices().then((d) => alive && setDevices(d)).catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  // El medidor solo tiene sentido con el micrófono escuchando (configuración guardada y activa).
  useEffect(() => {
    if (!saved.enabled) {
      setStatus(null);
      return;
    }
    let alive = true;
    const tick = () => void api.getMicStatus().then((s) => alive && setStatus(s)).catch(() => undefined);
    tick();
    const id = setInterval(tick, 120);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [saved.enabled, saved.device, saved.thresholdDb, saved.holdMs]);

  const unsaved = JSON.stringify(value) !== JSON.stringify(saved);

  return (
    <Card title={t("tts.mic.title")} hint={t("tts.mic.hint")}>
      <div className="space-y-3">
        <Checkbox checked={value.enabled} onChange={(enabled) => onChange({ enabled })} label={t("tts.mic.enabled")} />
        {value.enabled && (
          <>
            <div className="grid grid-cols-2 gap-3">
              <Field label={t("tts.mic.mode")}>
                <Select<MicGuardMode>
                  value={value.mode}
                  onChange={(mode) => onChange({ mode })}
                  options={[
                    { value: "repeatWord", label: t("tts.mic.repeatWord") },
                    { value: "repeatMessage", label: t("tts.mic.repeatMessage") },
                    { value: "skip", label: t("tts.mic.skip") },
                  ]}
                />
              </Field>
              <Field label={t("tts.mic.device")}>
                <Select
                  value={value.device ?? ""}
                  onChange={(d) => onChange({ device: d || null })}
                  options={[{ value: "", label: t("tts.mic.defaultDevice") }, ...devices.map((d) => ({ value: d, label: d }))]}
                />
              </Field>
            </div>
            <div className="grid grid-cols-3 gap-3">
              <Field label={t("tts.mic.sensitivity", { db: value.thresholdDb })} hint={t("tts.mic.sensitivityHint")} className="col-span-2">
                <input
                  type="range"
                  min={MIN_DB}
                  max={-5}
                  step={1}
                  value={value.thresholdDb}
                  onChange={(e) => onChange({ thresholdDb: Number(e.target.value) })}
                  className="w-full accent-amber-400"
                />
              </Field>
              <Field label={t("tts.mic.hold")} hint={t("tts.mic.holdHint")}>
                <NumberInput value={value.holdMs} min={200} max={5000} step={100} onChange={(v) => onChange({ holdMs: v ?? 800 })} />
              </Field>
            </div>
            <div>
              <div className="relative h-3 overflow-hidden rounded bg-zinc-800">
                <div
                  className={`h-full transition-[width] duration-100 ${status?.speaking ? "bg-emerald-400" : "bg-zinc-500"}`}
                  style={{ width: `${status ? pct(status.levelDb) : 0}%` }}
                />
                <div className="absolute top-0 h-full w-0.5 bg-amber-400" style={{ left: `${pct(value.thresholdDb)}%` }} title={t("tts.mic.threshold")} />
              </div>
              <p className="mt-1 text-xs text-zinc-500">
                {status?.error
                  ? <span className="text-red-400">{status.error}</span>
                  : unsaved || !status
                    ? t("tts.mic.saveToTest")
                    : status.speaking
                      ? <span className="text-emerald-400">{t("tts.mic.talking")}</span>
                      : t("tts.mic.quiet", { db: Math.round(status.levelDb) })}
              </p>
            </div>
            <p className="text-xs text-amber-300/80">{t("tts.mic.headphones")}</p>
          </>
        )}
      </div>
    </Card>
  );
}
