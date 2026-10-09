import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Extension, ExtensionSource, TimerConfig } from "../../lib/types";
import { Btn, Card, ErrorText, Field, NumberInput, Select, TextInput, useAction } from "../ui";

const SOURCES: ExtensionSource["type"][] = ["coins", "likes", "follow", "share", "subscribe", "gift"];

export function newTimer(): TimerConfig {
  return { id: crypto.randomUUID(), name: "", startSeconds: 3600, extensions: [] };
}

function defaultSource(type: ExtensionSource["type"]): ExtensionSource {
  switch (type) {
    case "coins":
      return { type, perCoins: 100 };
    case "likes":
      return { type, perLikes: 100 };
    case "gift":
      return { type, giftName: "" };
    default:
      return { type };
  }
}

interface Props {
  initial: TimerConfig;
  isNew: boolean;
  onSave: (c: TimerConfig) => Promise<void>;
  onCancel: () => void;
}

export function TimerEditor({ initial, isNew, onSave, onCancel }: Props) {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<TimerConfig>(initial);
  const { run, error, busy } = useAction();
  const setExts = (extensions: Extension[]) => setCfg((c) => ({ ...c, extensions }));
  const patchExt = (i: number, p: Partial<Extension>) => setExts(cfg.extensions.map((e, j) => (j === i ? { ...e, ...p } : e)));

  return (
    <Card title={isNew ? t("timers.newTitle") : t("timers.editTitle")}>
      <div className="space-y-3">
        <div className="grid grid-cols-3 gap-3">
          <Field label={t("goals.name")}>
            <TextInput value={cfg.name} onChange={(name) => setCfg((c) => ({ ...c, name }))} placeholder={t("timers.namePlaceholder")} />
          </Field>
          <Field label={t("timers.startMinutes")}>
            <NumberInput value={cfg.startSeconds / 60} min={1} step={1} onChange={(v) => setCfg((c) => ({ ...c, startSeconds: Math.max(1, Math.round((v ?? 1) * 60)) }))} />
          </Field>
          <Field label={t("timers.maxMinutes")} hint={t("timers.maxHint")}>
            <NumberInput
              value={cfg.maxSeconds !== undefined ? cfg.maxSeconds / 60 : null}
              min={1}
              onChange={(v) =>
                setCfg((c) => {
                  const { maxSeconds: _drop, ...rest } = c;
                  return v === null ? rest : { ...rest, maxSeconds: Math.max(1, Math.round(v * 60)) };
                })
              }
            />
          </Field>
        </div>

        <div>
          <h3 className="mb-1 text-xs font-semibold text-zinc-400">{t("timers.extensions")}</h3>
          <p className="mb-2 text-xs text-zinc-500">{t("timers.extensionsHint")}</p>
          <div className="space-y-2">
            {cfg.extensions.map((e, i) => (
              <div key={i} className="flex items-end gap-2 rounded-lg border border-zinc-800 bg-zinc-950/50 p-2">
                <Field label={t("timers.source")} className="w-44">
                  <Select
                    value={e.source.type}
                    onChange={(type) => patchExt(i, { source: defaultSource(type) })}
                    options={SOURCES.map((s) => ({ value: s, label: t(`timers.sources.${s}`) }))}
                  />
                </Field>
                {e.source.type === "coins" && (
                  <Field label={t("timers.perCoins")} className="w-28">
                    <NumberInput value={e.source.perCoins} min={1} onChange={(v) => patchExt(i, { source: { type: "coins", perCoins: Math.max(1, v ?? 1) } })} />
                  </Field>
                )}
                {e.source.type === "likes" && (
                  <Field label={t("timers.perLikes")} className="w-28">
                    <NumberInput value={e.source.perLikes} min={1} onChange={(v) => patchExt(i, { source: { type: "likes", perLikes: Math.max(1, v ?? 1) } })} />
                  </Field>
                )}
                {e.source.type === "gift" && (
                  <Field label={t("trigger.giftName")} className="w-40">
                    <TextInput value={e.source.giftName ?? ""} onChange={(v) => patchExt(i, { source: { type: "gift", giftName: v } })} />
                  </Field>
                )}
                <Field label={t("timers.addSeconds")} className="w-28">
                  <NumberInput value={e.seconds} min={1} onChange={(v) => patchExt(i, { seconds: Math.max(1, v ?? 1) })} />
                </Field>
                <Btn variant="danger" onClick={() => setExts(cfg.extensions.filter((_, j) => j !== i))}>✕</Btn>
              </div>
            ))}
            <Btn onClick={() => setExts([...cfg.extensions, { source: { type: "coins", perCoins: 100 }, seconds: 60 }])}>+ {t("timers.addExtension")}</Btn>
          </div>
        </div>

        <p className="text-xs text-zinc-500">
          {t("goals.idHint")} <code className="rounded bg-zinc-950 px-1">{cfg.id}</code>
        </p>

        <div className="flex items-center justify-end gap-2">
          <ErrorText error={error} />
          <Btn onClick={onCancel}>{t("editor.cancel")}</Btn>
          <Btn variant="primary" disabled={busy} onClick={() => void run(() => onSave(cfg))}>
            {t("editor.save")}
          </Btn>
        </div>
      </div>
    </Card>
  );
}
