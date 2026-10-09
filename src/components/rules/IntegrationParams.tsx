import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { ActionSpec } from "../../lib/types";
import { Checkbox, Field, NumberInput, Select, TextInput } from "../ui";
import { amountValue, bool, listValue, num, set, str, toAmount, toList } from "./paramHelpers";

interface P {
  action: ActionSpec;
  onChange: (a: ActionSpec) => void;
}

/** Campo JSON libre: solo propaga el valor cuando el texto es JSON válido. */
function JsonField({ label, hint, value, onValid }: { label: string; hint?: string; value: unknown; onValid: (v: unknown) => void }) {
  const { t } = useTranslation();
  const [text, setText] = useState(() => (value === undefined ? "" : JSON.stringify(value, null, 2)));
  const [error, setError] = useState<string | null>(null);
  return (
    <Field label={label} hint={error ?? hint}>
      <textarea
        value={text}
        rows={3}
        spellCheck={false}
        placeholder="{}"
        onChange={(e) => {
          setText(e.target.value);
          if (e.target.value.trim() === "") {
            setError(null);
            return onValid(null);
          }
          try {
            onValid(JSON.parse(e.target.value));
            setError(null);
          } catch {
            setError(t("action.jsonInvalid"));
          }
        }}
        className="w-full rounded-md border border-zinc-700 bg-zinc-950 p-2 font-mono text-xs outline-none focus:border-amber-400"
      />
    </Field>
  );
}

export function BotMessageParams({ action, onChange }: P) {
  const { t } = useTranslation();
  return (
    <Field label={t("action.botText")} hint={t("action.botTextHint")}>
      <TextInput value={str(action, "text")} onChange={(v) => onChange(set(action, "text", v))} placeholder="¡Gracias {nickname}!" />
    </Field>
  );
}

export function PointsAdjustParams({ action, onChange }: P) {
  const { t } = useTranslation();
  return (
    <div className="grid grid-cols-2 gap-2">
      <Field label={t("action.pointsAmount")} hint={t("action.pointsAmountHint")}>
        <TextInput value={amountValue(action, "amount")} onChange={(v) => onChange(set(action, "amount", toAmount(v)))} placeholder="10" />
      </Field>
      <Field label={t("action.pointsTarget")} hint={t("action.pointsTargetHint")}>
        <TextInput value={str(action, "target")} onChange={(v) => onChange(set(action, "target", v))} placeholder="{user}" />
      </Field>
    </div>
  );
}

export function StartPollParams({ action, onChange }: P) {
  const { t } = useTranslation();
  const options = Array.isArray(action.options) ? (action.options as string[]) : [];
  return (
    <div className="space-y-2">
      <Field label={t("action.pollQuestion")}>
        <TextInput value={str(action, "question")} onChange={(v) => onChange(set(action, "question", v))} />
      </Field>
      <div className="grid grid-cols-3 gap-2">
        <Field label={t("action.pollOptions")} hint={t("action.pollOptionsHint")} className="col-span-2">
          <textarea
            value={options.join("\n")}
            rows={3}
            onChange={(e) => onChange({ ...action, options: e.target.value.split("\n") })}
            className="w-full rounded-md border border-zinc-700 bg-zinc-950 p-2 text-sm outline-none focus:border-amber-400"
          />
        </Field>
        <Field label={t("action.pollDuration")}>
          <NumberInput value={num(action, "durationSec") ?? 60} min={5} max={3600} onChange={(v) => onChange(set(action, "durationSec", v))} />
        </Field>
      </div>
    </div>
  );
}

export function WebhookParams({ action, onChange }: P) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-4 gap-2">
        <Field label={t("action.webhookUrl")} hint={t("action.webhookUrlHint")} className="col-span-3">
          <TextInput value={str(action, "url")} onChange={(v) => onChange(set(action, "url", v))} placeholder="https://…/{user}" />
        </Field>
        <Field label={t("action.webhookMethod")}>
          <Select
            value={str(action, "method") || "GET"}
            onChange={(v) => onChange(set(action, "method", v))}
            options={["GET", "POST", "PUT", "PATCH", "DELETE"].map((m) => ({ value: m, label: m }))}
          />
        </Field>
      </div>
      <JsonField label={t("action.webhookBodyJson")} hint={t("action.webhookBodyHint")} value={action.bodyJson} onValid={(v) => onChange(set(action, "bodyJson", v))} />
      <JsonField label={t("action.webhookHeaders")} hint={t("action.webhookHeadersHint")} value={action.headers} onValid={(v) => onChange(set(action, "headers", v))} />
    </div>
  );
}

export function TcpSendParams({ action, onChange }: P) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-3 gap-2">
        <Field label={t("action.host")}>
          <TextInput value={str(action, "host")} onChange={(v) => onChange(set(action, "host", v))} placeholder="127.0.0.1" />
        </Field>
        <Field label={t("action.port")}>
          <NumberInput value={num(action, "port")} min={1} max={65535} onChange={(v) => onChange(set(action, "port", v))} />
        </Field>
        <div className="flex items-end pb-1.5">
          <Checkbox checked={bool(action, "newline", true)} onChange={(v) => onChange(set(action, "newline", v))} label={t("action.newline")} />
        </div>
      </div>
      <Field label={t("action.netMessage")} hint={t("action.netMessageHint")}>
        <TextInput value={str(action, "message")} onChange={(v) => onChange(set(action, "message", v))} placeholder="spawn {nickname}" />
      </Field>
    </div>
  );
}

export function WsSendParams({ action, onChange }: P) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <Field label={t("action.wsUrl")}>
        <TextInput value={str(action, "url")} onChange={(v) => onChange(set(action, "url", v))} placeholder="ws://127.0.0.1:8080" />
      </Field>
      <Field label={t("action.netMessage")} hint={t("action.netMessageHint")}>
        <TextInput value={str(action, "message")} onChange={(v) => onChange(set(action, "message", v))} placeholder='{"user":"{nickname}"}' />
      </Field>
    </div>
  );
}

export function PressKeysParams({ action, onChange }: P) {
  const { t } = useTranslation();
  const anyWindow = bool(action, "anyWindow", false);
  const keys = Array.isArray(action.keys) ? listValue(action, "keys") : str(action, "keys");
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-3 gap-2">
        <Field label={t("action.keys")} hint={t("action.keysHint")} className="col-span-2">
          <TextInput value={keys} onChange={(v) => onChange(set(action, "keys", toList(v).length > 1 ? toList(v) : v.trim()))} placeholder="ctrl+shift+f5" />
        </Field>
        <Field label={t("action.holdMs")}>
          <NumberInput value={num(action, "holdMs") ?? 50} min={0} max={10000} onChange={(v) => onChange(set(action, "holdMs", v))} />
        </Field>
      </div>
      <Field label={t("action.targetWindows")} hint={t("action.targetWindowsHint")}>
        <TextInput value={listValue(action, "targetWindows")} onChange={(v) => onChange(set(action, "targetWindows", toList(v)))} placeholder="Minecraft, obs64.exe" />
      </Field>
      <Checkbox checked={anyWindow} onChange={(v) => onChange(set(action, "anyWindow", v ? true : null))} label={t("action.anyWindow")} />
      {anyWindow && <p className="text-xs text-amber-400">{t("action.anyWindowWarning")}</p>}
    </div>
  );
}

const OBS_ACTIONS = ["setScene", "setSourceVisible", "setFilterEnabled", "startRecording", "stopRecording"] as const;

export function ObsParams({ action, onChange }: P) {
  const { t } = useTranslation();
  const kind = str(action, "action") || "setScene";
  const reverts = kind === "setSourceVisible" || kind === "setFilterEnabled";
  const flag = kind === "setSourceVisible" ? "visible" : "enabled";
  return (
    <div className="space-y-2">
      <Field label={t("action.obsAction")}>
        <Select value={kind} onChange={(v) => onChange({ type: "obs", action: v })} options={OBS_ACTIONS.map((o) => ({ value: o, label: t(`action.obsActions.${o}`) }))} />
      </Field>
      <div className="grid grid-cols-3 gap-2">
        {(kind === "setScene" || kind === "setSourceVisible") && (
          <Field label={t("action.obsScene")}>
            <TextInput value={str(action, "scene")} onChange={(v) => onChange(set(action, "scene", v))} />
          </Field>
        )}
        {(kind === "setSourceVisible" || kind === "setFilterEnabled") && (
          <Field label={t("action.obsSource")}>
            <TextInput value={str(action, "source")} onChange={(v) => onChange(set(action, "source", v))} />
          </Field>
        )}
        {kind === "setFilterEnabled" && (
          <Field label={t("action.obsFilter")}>
            <TextInput value={str(action, "filter")} onChange={(v) => onChange(set(action, "filter", v))} />
          </Field>
        )}
      </div>
      {reverts && (
        <div className="grid grid-cols-3 gap-2">
          <div className="flex items-end pb-1.5">
            <Checkbox
              checked={bool(action, flag, true)}
              onChange={(v) => onChange(set(action, flag, v))}
              label={kind === "setSourceVisible" ? t("action.obsVisible") : t("action.obsEnabled")}
            />
          </div>
          <Field label={t("action.obsRevert")} hint={t("action.obsRevertHint")}>
            <NumberInput value={num(action, "durationMs")} min={1} max={600000} step={500} onChange={(v) => onChange(set(action, "durationMs", v))} />
          </Field>
        </div>
      )}
    </div>
  );
}
