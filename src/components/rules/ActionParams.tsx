import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { ActionSpec, Goal, Media, Sound, TimerView, VoiceInfo } from "../../lib/types";
import { voiceLabel } from "../../lib/voices";
import { Checkbox, Field, NumberInput, Select, TextInput } from "../ui";
import { amountValue, bool, num, set, str, toAmount } from "./paramHelpers";
import { BotMessageParams, ObsParams, PointsAdjustParams, PressKeysParams, StartPollParams, TcpSendParams, WebhookParams, WsSendParams } from "./IntegrationParams";

interface Props {
  action: ActionSpec;
  onChange: (a: ActionSpec) => void;
  sounds: Sound[];
  media: Media[];
  voices: VoiceInfo[];
  goals: Goal[];
  timers: TimerView[];
}

export function ActionParams({ action, onChange, sounds, media, voices, goals, timers }: Props) {
  switch (action.type) {
    case "playSound":
      return <PlaySoundParams {...{ action, onChange, sounds }} />;
    case "overlayAlert":
      return <AlertParams {...{ action, onChange, media }} />;
    case "tts":
      return <TtsParams {...{ action, onChange, voices }} />;
    case "goalAdjust":
      return <GoalAdjustParams {...{ action, onChange, goals }} />;
    case "timerControl":
      return <TimerControlParams {...{ action, onChange, timers }} />;
    case "botMessage":
      return <BotMessageParams {...{ action, onChange }} />;
    case "pointsAdjust":
      return <PointsAdjustParams {...{ action, onChange }} />;
    case "startPoll":
      return <StartPollParams {...{ action, onChange }} />;
    case "webhook":
      return <WebhookParams {...{ action, onChange }} />;
    case "tcpSend":
      return <TcpSendParams {...{ action, onChange }} />;
    case "wsSend":
      return <WsSendParams {...{ action, onChange }} />;
    case "pressKeys":
      return <PressKeysParams {...{ action, onChange }} />;
    case "obs":
      return <ObsParams {...{ action, onChange }} />;
    case "spinWheel":
      return <NoParams />;
    default:
      return <GenericParams action={action} onChange={onChange} />;
  }
}

function GoalAdjustParams({ action, onChange, goals }: Pick<Props, "action" | "onChange" | "goals">) {
  const { t } = useTranslation();
  const op = str(action, "op") || "add";
  return (
    <div className="grid grid-cols-3 gap-2">
      <Field label={t("action.goal")}>
        <Select
          value={str(action, "goalId")}
          onChange={(v) => onChange(set(action, "goalId", v))}
          options={[{ value: "", label: t("action.chooseGoal") }, ...goals.map((g) => ({ value: g.id, label: g.name }))]}
        />
      </Field>
      <Field label={t("action.operation")}>
        <Select
          value={op}
          onChange={(v) => onChange(set(action, "op", v))}
          options={[
            { value: "add", label: t("action.goalOps.add") },
            { value: "set", label: t("action.goalOps.set") },
            { value: "reset", label: t("action.goalOps.reset") },
          ]}
        />
      </Field>
      {op !== "reset" && (
        <Field label={t("action.amount")} hint={t("action.amountHint")}>
          <TextInput value={amountValue(action, "amount")} onChange={(v) => onChange(set(action, "amount", toAmount(v)))} placeholder="{count}" />
        </Field>
      )}
    </div>
  );
}

function TimerControlParams({ action, onChange, timers }: Pick<Props, "action" | "onChange" | "timers">) {
  const { t } = useTranslation();
  const op = str(action, "op") || "add";
  return (
    <div className="grid grid-cols-3 gap-2">
      <Field label={t("action.timer")}>
        <Select
          value={str(action, "timerId")}
          onChange={(v) => onChange(set(action, "timerId", v))}
          options={[{ value: "", label: t("action.chooseTimer") }, ...timers.map((tm) => ({ value: tm.config.id, label: tm.config.name }))]}
        />
      </Field>
      <Field label={t("action.operation")}>
        <Select
          value={op}
          onChange={(v) => onChange(set(action, "op", v))}
          options={["start", "pause", "resume", "reset", "add"].map((o) => ({ value: o, label: t(`action.timerOps.${o}`) }))}
        />
      </Field>
      {op === "add" && (
        <Field label={t("action.seconds")} hint={t("action.amountHint")}>
          <TextInput value={amountValue(action, "seconds")} onChange={(v) => onChange(set(action, "seconds", toAmount(v)))} placeholder="{coins}" />
        </Field>
      )}
    </div>
  );
}

function PlaySoundParams({ action, onChange, sounds }: Pick<Props, "action" | "onChange" | "sounds">) {
  const { t } = useTranslation();
  const selected = Array.isArray(action.soundIds) ? (action.soundIds as string[]) : str(action, "soundId") ? [str(action, "soundId")] : [];

  function toggle(id: string, on: boolean) {
    const next = on ? [...selected, id] : selected.filter((s) => s !== id);
    // Un solo sonido se guarda como `soundId`; varios, como `soundIds` (se elige uno al azar).
    const base = set(set(action, "soundId", null), "soundIds", null);
    onChange(next.length <= 1 ? set(base, "soundId", next[0] ?? null) : set(base, "soundIds", next));
  }

  return (
    <div className="space-y-2">
      <Field label={t("action.sounds")} hint={t("action.soundsHint")}>
        {sounds.length === 0 ? (
          <p className="text-zinc-500">{t("action.noSounds")}</p>
        ) : (
          <div className="grid max-h-32 grid-cols-2 gap-1 overflow-y-auto rounded-md border border-zinc-800 p-2">
            {sounds.map((s) => (
              <Checkbox key={s.id} checked={selected.includes(s.id)} onChange={(v) => toggle(s.id, v)} label={s.name} />
            ))}
          </div>
        )}
      </Field>
      <div className="grid grid-cols-2 gap-2">
        <Field label={t("action.volume")}>
          <NumberInput value={num(action, "volume") ?? 100} min={0} max={100} onChange={(v) => onChange(set(action, "volume", v))} />
        </Field>
        <div className="flex items-end pb-1.5">
          <Checkbox checked={bool(action, "wait", true)} onChange={(v) => onChange(set(action, "wait", v))} label={t("action.wait")} />
        </div>
      </div>
    </div>
  );
}

function AlertParams({ action, onChange, media }: Pick<Props, "action" | "onChange" | "media">) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2">
        <Field label={t("action.alertTitle")}>
          <TextInput value={str(action, "title")} onChange={(v) => onChange(set(action, "title", v))} placeholder="{nickname}" />
        </Field>
        <Field label={t("action.alertText")}>
          <TextInput value={str(action, "text")} onChange={(v) => onChange(set(action, "text", v))} placeholder="envió {count}× {gift}" />
        </Field>
      </div>
      <div className="grid grid-cols-2 gap-2">
        <Field label={t("action.media")}>
          <Select
            value={str(action, "mediaId")}
            onChange={(v) => onChange(set(action, "mediaId", v))}
            options={[{ value: "", label: t("action.noMedia") }, ...media.map((m) => ({ value: m.id, label: `${m.kind === "video" ? "🎬" : "🖼️"} ${m.name}` }))]}
          />
        </Field>
        <Field label={t("action.imageUrl")} hint={t("action.imageUrlHint")}>
          <TextInput value={str(action, "imageUrl")} onChange={(v) => onChange(set(action, "imageUrl", v))} placeholder="{giftimage}" />
        </Field>
      </div>
      <div className="grid grid-cols-3 gap-2">
        <Field label={t("action.duration")}>
          <NumberInput value={num(action, "durationMs") ?? 5000} min={500} max={60000} step={500} onChange={(v) => onChange(set(action, "durationMs", v))} />
        </Field>
        <div className="flex items-end pb-1.5">
          <Checkbox checked={bool(action, "showAvatar", false)} onChange={(v) => onChange(set(action, "showAvatar", v))} label={t("action.showAvatar")} />
        </div>
        <div className="flex items-end pb-1.5">
          <Checkbox checked={bool(action, "wait", true)} onChange={(v) => onChange(set(action, "wait", v))} label={t("action.wait")} />
        </div>
      </div>
    </div>
  );
}

function TtsParams({ action, onChange, voices }: Pick<Props, "action" | "onChange" | "voices">) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <Field label={t("action.ttsText")}>
        <TextInput value={str(action, "text")} onChange={(v) => onChange(set(action, "text", v))} placeholder="Gracias {nickname} por {count} {gift}" />
      </Field>
      <div className="grid grid-cols-3 gap-2">
        <Field label={t("action.voice")}>
          <Select
            value={str(action, "voice")}
            onChange={(v) => onChange(set(action, "voice", v))}
            options={[{ value: "", label: t("action.autoVoice") }, ...voices.map((v) => ({ value: v.id, label: voiceLabel(v) }))]}
          />
        </Field>
        <Field label={t("action.rate")}>
          <NumberInput value={num(action, "rate")} min={0.5} max={2} step={0.1} placeholder="1.0" onChange={(v) => onChange(set(action, "rate", v))} />
        </Field>
        <Field label={t("action.volume")}>
          <NumberInput value={num(action, "volume")} min={0} max={100} placeholder="100" onChange={(v) => onChange(set(action, "volume", v))} />
        </Field>
      </div>
    </div>
  );
}

/** Editor genérico (JSON) para tipos de acción que aún no tienen formulario propio. */
function GenericParams({ action, onChange }: Pick<Props, "action" | "onChange">) {
  const { t } = useTranslation();
  const { type: _type, ...params } = action;
  const [text, setText] = useState(() => JSON.stringify(params, null, 2));
  const [error, setError] = useState<string | null>(null);

  return (
    <Field label={t("action.json")} hint={error ?? t("action.jsonHint")}>
      <textarea
        value={text}
        rows={5}
        spellCheck={false}
        onChange={(e) => {
          setText(e.target.value);
          try {
            const parsed: unknown = JSON.parse(e.target.value);
            if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) throw new Error(t("action.jsonObject"));
            setError(null);
            onChange({ ...(parsed as Record<string, unknown>), type: action.type });
          } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
          }
        }}
        className="w-full rounded-md border border-zinc-700 bg-zinc-950 p-2 font-mono text-xs outline-none focus:border-amber-400"
      />
    </Field>
  );
}

function NoParams() {
  const { t } = useTranslation();
  return <p className="text-xs text-zinc-500">{t("action.spinWheelHint")}</p>;
}
