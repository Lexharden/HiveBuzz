import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Goal, GoalKind, OnReach } from "../../lib/types";
import { Btn, Card, Checkbox, ErrorText, Field, NumberInput, Select, TextInput, useAction } from "../ui";

const KINDS: GoalKind["type"][] = ["likes", "follows", "shares", "subscribers", "coins", "gift"];
const ON_REACH: OnReach["type"][] = ["stop", "reset", "extend"];

export function newGoal(): Goal {
  return {
    id: crypto.randomUUID(),
    name: "",
    kind: { type: "likes" },
    target: 1000,
    current: 0,
    onReach: { type: "stop" },
    resetOnSession: false,
    reachedCount: 0,
  };
}

interface Props {
  initial: Goal;
  isNew: boolean;
  onSave: (g: Goal) => Promise<void>;
  onCancel: () => void;
}

export function GoalEditor({ initial, isNew, onSave, onCancel }: Props) {
  const { t } = useTranslation();
  const [goal, setGoal] = useState<Goal>(initial);
  const { run, error, busy } = useAction();
  const patch = (p: Partial<Goal>) => setGoal((g) => ({ ...g, ...p }));

  function setKind(type: GoalKind["type"]) {
    patch({ kind: type === "gift" ? { type, giftName: "" } : ({ type } as GoalKind) });
  }
  function setOnReach(type: OnReach["type"]) {
    patch({ onReach: type === "extend" ? { type, add: Math.max(1, Math.round(goal.target / 2)) } : ({ type } as OnReach) });
  }

  return (
    <Card title={isNew ? t("goals.newTitle") : t("goals.editTitle")}>
      <div className="space-y-3">
        <div className="grid grid-cols-3 gap-3">
          <Field label={t("goals.name")} className="col-span-2">
            <TextInput value={goal.name} onChange={(name) => patch({ name })} placeholder={t("goals.namePlaceholder")} />
          </Field>
          <Field label={t("goals.target")}>
            <NumberInput value={goal.target} min={1} onChange={(v) => patch({ target: Math.max(1, v ?? 1) })} />
          </Field>
        </div>

        <div className="grid grid-cols-3 gap-3">
          <Field label={t("goals.kind")}>
            <Select value={goal.kind.type} onChange={setKind} options={KINDS.map((k) => ({ value: k, label: t(`goals.kinds.${k}`) }))} />
          </Field>
          {goal.kind.type === "gift" && (
            <>
              <Field label={t("trigger.giftName")}>
                <TextInput value={goal.kind.giftName ?? ""} onChange={(v) => patch({ kind: { type: "gift", giftName: v, ...(goal.kind.type === "gift" && goal.kind.giftId !== undefined ? { giftId: goal.kind.giftId } : {}) } })} />
              </Field>
              <Field label={t("trigger.giftId")}>
                <NumberInput
                  value={goal.kind.giftId ?? null}
                  onChange={(v) => patch({ kind: { type: "gift", ...(goal.kind.type === "gift" && goal.kind.giftName ? { giftName: goal.kind.giftName } : {}), ...(v === null ? {} : { giftId: v }) } })}
                />
              </Field>
            </>
          )}
        </div>

        <div className="grid grid-cols-3 gap-3">
          <Field label={t("goals.onReach")} hint={t(`goals.onReachHint.${goal.onReach.type}`)}>
            <Select value={goal.onReach.type} onChange={setOnReach} options={ON_REACH.map((k) => ({ value: k, label: t(`goals.onReachOptions.${k}`) }))} />
          </Field>
          {goal.onReach.type === "extend" && (
            <Field label={t("goals.extendBy")}>
              <NumberInput value={goal.onReach.add} min={1} onChange={(v) => patch({ onReach: { type: "extend", add: Math.max(1, v ?? 1) } })} />
            </Field>
          )}
          <div className="flex items-end pb-1.5">
            <Checkbox checked={goal.resetOnSession} onChange={(resetOnSession) => patch({ resetOnSession })} label={t("goals.resetOnSession")} />
          </div>
        </div>

        <p className="text-xs text-zinc-500">
          {t("goals.idHint")} <code className="rounded bg-zinc-950 px-1">{goal.id}</code>
        </p>

        <div className="flex items-center justify-end gap-2">
          <ErrorText error={error} />
          <Btn onClick={onCancel}>{t("editor.cancel")}</Btn>
          <Btn variant="primary" disabled={busy} onClick={() => void run(() => onSave(goal))}>
            {t("editor.save")}
          </Btn>
        </div>
      </div>
    </Card>
  );
}
