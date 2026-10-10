import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Conditions, Goal, Media, Role, Rule, Sound, Step, TimerView, Trigger, TriggerType, VoiceInfo } from "../../lib/types";
import { Btn, Card, Checkbox, ErrorText, Field, NumberInput, Select, TextInput, useAction } from "../ui";
import { ActionParams } from "./ActionParams";
import { defaultAction, defaultTrigger, TRIGGER_TYPES } from "./ruleDefaults";

const ROLES: Role[] = ["moderator", "subscriber", "follower"];
const DAY_KEYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const;

interface Props {
  initial: Rule;
  isNew: boolean;
  actionTypes: string[];
  sounds: Sound[];
  media: Media[];
  voices: VoiceInfo[];
  goals: Goal[];
  timers: TimerView[];
  onSave: (rule: Rule) => Promise<void>;
  onCancel: () => void;
}

export function RuleEditor({ initial, isNew, actionTypes, sounds, media, voices, goals, timers, onSave, onCancel }: Props) {
  const { t } = useTranslation();
  const [rule, setRule] = useState<Rule>(initial);
  const { run, error, busy } = useAction();

  const patch = (p: Partial<Rule>) => setRule((r) => ({ ...r, ...p }));
  const patchCond = (p: Partial<Conditions>) => setRule((r) => ({ ...r, conditions: { ...r.conditions, ...p } }));
  const setSteps = (steps: Step[]) => setRule((r) => ({ ...r, plan: { ...r.plan, steps } }));
  const patchStep = (i: number, p: Partial<Step>) => setSteps(rule.plan.steps.map((s, j) => (j === i ? { ...s, ...p } : s)));
  const moveStep = (i: number, d: -1 | 1) => {
    const steps = [...rule.plan.steps];
    const k = i + d;
    if (k < 0 || k >= steps.length) return;
    [steps[i], steps[k]] = [steps[k] as Step, steps[i] as Step];
    setSteps(steps);
  };

  return (
    <div className="space-y-4">
      <Card title={isNew ? t("editor.newTitle") : t("editor.editTitle")}>
        <div className="grid grid-cols-3 gap-3">
          <Field label={t("editor.name")} className="col-span-2">
            <TextInput value={rule.name} onChange={(name) => patch({ name })} placeholder={t("editor.namePlaceholder")} />
          </Field>
          <div className="flex items-end pb-1.5">
            <Checkbox checked={rule.enabled} onChange={(enabled) => patch({ enabled })} label={t("rules.enabled")} />
          </div>
        </div>
      </Card>

      <Card title={t("editor.trigger")}>
        <Field label={t("editor.triggerType")}>
          <Select
            value={rule.trigger.type}
            onChange={(type: TriggerType) => patch({ trigger: defaultTrigger(type) })}
            options={TRIGGER_TYPES.map((v) => ({ value: v, label: t(`trigger.${v}`) }))}
          />
        </Field>
        <div className="mt-3">
          <TriggerFields trigger={rule.trigger} onChange={(trigger) => patch({ trigger })} goals={goals} timers={timers} />
        </div>
      </Card>

      <Card title={t("editor.conditions")} hint={t("editor.conditionsHint")}>
        <ConditionFields c={rule.conditions} onChange={patchCond} />
      </Card>

      <Card
        title={t("editor.plan")}
        hint={t("editor.variables")}
        actions={
          <div className="w-40">
            <Select
              value={rule.plan.mode}
              onChange={(mode) => setRule((r) => ({ ...r, plan: { ...r.plan, mode } }))}
              options={[
                { value: "sequence", label: t("editor.mode.sequence") },
                { value: "parallel", label: t("editor.mode.parallel") },
              ]}
            />
          </div>
        }
      >
        <div className="space-y-3">
          {rule.plan.steps.map((step, i) => (
            <div key={i} className="rounded-lg border border-zinc-800 bg-zinc-950/50 p-3">
              <div className="mb-2 flex items-end gap-2">
                <span className="pb-1.5 text-xs font-semibold text-zinc-500">{i + 1}</span>
                <Field label={t("editor.actionType")} className="flex-1">
                  <Select
                    value={step.action.type}
                    onChange={(type) => patchStep(i, { action: defaultAction(type) })}
                    options={actionTypes.map((a) => ({ value: a, label: t(`actionType.${a}`, { defaultValue: a }) }))}
                  />
                </Field>
                <Field label={t("editor.delay")} className="w-28">
                  <NumberInput value={step.delayMs} min={0} step={100} onChange={(v) => patchStep(i, { delayMs: v ?? 0 })} />
                </Field>
                <Btn onClick={() => moveStep(i, -1)} disabled={i === 0} title={t("editor.up")}>↑</Btn>
                <Btn onClick={() => moveStep(i, 1)} disabled={i === rule.plan.steps.length - 1} title={t("editor.down")}>↓</Btn>
                <Btn variant="danger" onClick={() => setSteps(rule.plan.steps.filter((_, j) => j !== i))} title={t("editor.removeStep")}>✕</Btn>
              </div>
              <ActionParams
                action={step.action}
                onChange={(action) => patchStep(i, { action })}
                {...{ sounds, media, voices, goals, timers }}
              />
            </div>
          ))}
          <Btn onClick={() => setSteps([...rule.plan.steps, { delayMs: 0, action: defaultAction(actionTypes[0] ?? "tts") }])}>
            + {t("editor.addStep")}
          </Btn>
        </div>
      </Card>

      <Card title={t("editor.queue")} hint={t("editor.queueHint")}>
        <div className="grid grid-cols-3 gap-3">
          <Field label={t("editor.costPoints")} hint={t("editor.costPointsHint")}>
            <NumberInput value={rule.costPoints ?? null} min={0} placeholder="0" onChange={(v) => setRule((r) => {
              const { costPoints: _c, ...rest } = r;
              return v === null || v <= 0 ? rest : { ...rest, costPoints: v };
            })} />
          </Field>
          <Field label={t("editor.priority")} hint={t("editor.priorityHint")}>
            <NumberInput value={rule.priority ?? null} min={-100} max={1000} placeholder={t("editor.auto")} onChange={(v) => setRule((r) => {
              const { priority: _p, ...rest } = r;
              return v === null ? rest : { ...rest, priority: v };
            })} />
          </Field>
          <Field label={t("editor.ttl")} hint={t("editor.ttlHint")}>
            <NumberInput value={Math.round(rule.ttlMs / 1000)} min={1} max={3600} onChange={(v) => patch({ ttlMs: Math.max(1, v ?? 60) * 1000 })} />
          </Field>
        </div>
      </Card>

      <div className="flex items-center justify-end gap-2">
        <ErrorText error={error} />
        <Btn onClick={onCancel}>{t("editor.cancel")}</Btn>
        <Btn variant="primary" disabled={busy} onClick={() => void run(() => onSave(rule))}>
          {t("editor.save")}
        </Btn>
      </div>
    </div>
  );
}

function TriggerFields({ trigger, onChange, goals, timers }: { trigger: Trigger; onChange: (t: Trigger) => void; goals: Goal[]; timers: TimerView[] }) {
  const { t } = useTranslation();
  switch (trigger.type) {
    case "gift":
      return (
        <div className="grid grid-cols-3 gap-2">
          <Field label={t("trigger.giftName")} hint={t("trigger.giftNameHint")}>
            <TextInput value={trigger.giftName ?? ""} onChange={(v) => onChange(v ? { ...trigger, giftName: v } : omit(trigger, "giftName"))} />
          </Field>
          <Field label={t("trigger.giftId")}>
            <NumberInput value={trigger.giftId ?? null} onChange={(v) => onChange(v === null ? omit(trigger, "giftId") : { ...trigger, giftId: v })} />
          </Field>
          <Field label={t("trigger.minCoins")} hint={t("trigger.minCoinsHint")}>
            <NumberInput value={trigger.minCoins ?? null} min={0} onChange={(v) => onChange(v === null ? omit(trigger, "minCoins") : { ...trigger, minCoins: v })} />
          </Field>
        </div>
      );
    case "like":
      return (
        <Field label={t("trigger.likeEvery")}>
          <NumberInput value={trigger.every} min={1} onChange={(v) => onChange({ ...trigger, every: Math.max(1, v ?? 1) })} />
        </Field>
      );
    case "command":
      return (
        <Field label={t("trigger.commandField")} hint={t("trigger.commandHint")}>
          <TextInput value={trigger.command} onChange={(command) => onChange({ ...trigger, command })} />
        </Field>
      );
    case "keyword":
      return (
        <div className="space-y-2">
          <Field label={t("trigger.keywords")} hint={t("trigger.keywordsHint")}>
            <TextInput
              value={trigger.keywords.join(", ")}
              onChange={(v) => onChange({ ...trigger, keywords: v.split(",").map((k) => k.trim()).filter(Boolean) })}
            />
          </Field>
          <Checkbox checked={trigger.wholeWord} onChange={(wholeWord) => onChange({ ...trigger, wholeWord })} label={t("trigger.wholeWord")} />
        </div>
      );
    case "goalReached":
      return (
        <Field label={t("trigger.goalId")}>
          <Select
            value={trigger.goalId}
            onChange={(goalId) => onChange({ ...trigger, goalId })}
            options={[{ value: "", label: t("action.chooseGoal") }, ...goals.map((g) => ({ value: g.id, label: g.name }))]}
          />
        </Field>
      );
    case "timerEnded":
      return (
        <Field label={t("trigger.timerId")}>
          <Select
            value={trigger.timerId}
            onChange={(timerId) => onChange({ ...trigger, timerId })}
            options={[{ value: "", label: t("action.chooseTimer") }, ...timers.map((tm) => ({ value: tm.config.id, label: tm.config.name }))]}
          />
        </Field>
      );
    case "api":
      return (
        <Field label={t("trigger.apiName")} hint={t("trigger.apiHint")}>
          <TextInput value={trigger.name} onChange={(name) => onChange({ ...trigger, name: name.replace(/[^A-Za-z0-9_-]/g, "") })} />
        </Field>
      );
    default:
      return <p className="text-xs text-zinc-500">{t("trigger.noParams")}</p>;
  }
}

function omit<T extends object, K extends keyof T>(obj: T, key: K): Omit<T, K> {
  const { [key]: _removed, ...rest } = obj;
  return rest;
}

function ConditionFields({ c, onChange }: { c: Conditions; onChange: (p: Partial<Conditions>) => void }) {
  const { t } = useTranslation();
  const toggleRole = (r: Role, on: boolean) => onChange({ rolesAny: on ? [...c.rolesAny, r] : c.rolesAny.filter((x) => x !== r) });
  const sched = c.schedule;
  const toggleDay = (d: number, on: boolean) => {
    if (!sched) return;
    onChange({ schedule: { ...sched, days: on ? [...sched.days, d].sort() : sched.days.filter((x) => x !== d) } });
  };

  return (
    <div className="space-y-3">
      <div className="grid grid-cols-3 gap-2">
        <Field label={t("cond.globalCooldown")}>
          <NumberInput value={c.globalCooldownMs / 1000} min={0} step={1} onChange={(v) => onChange({ globalCooldownMs: Math.round((v ?? 0) * 1000) })} />
        </Field>
        <Field label={t("cond.userCooldown")}>
          <NumberInput value={c.userCooldownMs / 1000} min={0} step={1} onChange={(v) => onChange({ userCooldownMs: Math.round((v ?? 0) * 1000) })} />
        </Field>
        <Field label={t("cond.probability")}>
          <NumberInput value={c.probability} min={0} max={100} onChange={(v) => onChange({ probability: v ?? 100 })} />
        </Field>
      </div>

      <Field label={t("cond.roles")} hint={t("cond.rolesHint")}>
        <div className="flex gap-4">
          {ROLES.map((r) => (
            <Checkbox key={r} checked={c.rolesAny.includes(r)} onChange={(on) => toggleRole(r, on)} label={t(`role.${r}`)} />
          ))}
        </div>
      </Field>

      <div className="grid grid-cols-2 gap-2">
        <Field label={t("cond.minTeam")}>
          <NumberInput value={c.minTeamLevel ?? null} min={0} onChange={(v) => onChange({ minTeamLevel: v ?? undefined })} />
        </Field>
        <Field label={t("cond.minGifter")}>
          <NumberInput value={c.minGifterLevel ?? null} min={0} onChange={(v) => onChange({ minGifterLevel: v ?? undefined })} />
        </Field>
      </div>

      <div>
        <Checkbox
          checked={sched !== undefined}
          onChange={(on) => onChange({ schedule: on ? { days: [], from: "20:00", to: "23:59" } : undefined })}
          label={t("cond.schedule")}
        />
        {sched && (
          <div className="mt-2 flex flex-wrap items-end gap-3">
            <Field label={t("cond.from")}>
              <input type="time" value={sched.from} onChange={(e) => onChange({ schedule: { ...sched, from: e.target.value } })}
                className="rounded-md border border-zinc-700 bg-zinc-950 px-2 py-1.5 text-sm" />
            </Field>
            <Field label={t("cond.to")}>
              <input type="time" value={sched.to} onChange={(e) => onChange({ schedule: { ...sched, to: e.target.value } })}
                className="rounded-md border border-zinc-700 bg-zinc-950 px-2 py-1.5 text-sm" />
            </Field>
            <div className="flex gap-3 pb-1.5">
              {DAY_KEYS.map((k, d) => (
                <Checkbox key={k} checked={sched.days.includes(d)} onChange={(on) => toggleDay(d, on)} label={t(`day.${k}`)} />
              ))}
            </div>
            <p className="pb-2 text-xs text-zinc-500">{t("cond.daysHint")}</p>
          </div>
        )}
      </div>
    </div>
  );
}
