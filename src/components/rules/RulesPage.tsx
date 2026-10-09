import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage, onRuleFired } from "../../lib/api";
import type { Goal, Media, QueueInfo, Rule, Sound, TimerView, VoiceInfo } from "../../lib/types";
import { Btn, Card, ErrorText } from "../ui";
import { newRule, triggerSummary } from "./ruleDefaults";
import { RuleEditor } from "./RuleEditor";

export function RulesPage() {
  const { t } = useTranslation();
  const [rules, setRules] = useState<Rule[]>([]);
  const [actionTypes, setActionTypes] = useState<string[]>([]);
  const [sounds, setSounds] = useState<Sound[]>([]);
  const [media, setMedia] = useState<Media[]>([]);
  const [voices, setVoices] = useState<VoiceInfo[]>([]);
  const [goals, setGoals] = useState<Goal[]>([]);
  const [timers, setTimers] = useState<TimerView[]>([]);
  const [editing, setEditing] = useState<{ rule: Rule; isNew: boolean } | null>(null);
  const [queue, setQueue] = useState<QueueInfo>({ pending: 0, running: 0, jobs: 0 });
  const [flash, setFlash] = useState<Record<string, number>>({});
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);

  const reload = useCallback(async () => {
    try {
      const [r, a, s, m, v, g, tm] = await Promise.all([
        api.listRules(),
        api.listActionTypes(),
        api.listSounds(),
        api.listMedia(),
        api.listTtsVoices(),
        api.listGoals(),
        api.listTimers(),
      ]);
      if (!alive.current) return;
      setRules(r);
      setActionTypes(a);
      setSounds(s);
      setMedia(m);
      setVoices(v);
      setGoals(g);
      setTimers(tm);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    void reload();
    const poll = setInterval(() => void api.queueStats().then((q) => alive.current && setQueue(q)).catch(() => undefined), 1500);
    let off: (() => void) | undefined;
    void onRuleFired((r) => {
      setFlash((f) => ({ ...f, [r.ruleId]: Date.now() }));
      setTimeout(() => setFlash((f) => (f[r.ruleId] === undefined ? f : omitKey(f, r.ruleId))), 1500);
    }).then((u) => (alive.current ? (off = u) : u()));
    return () => {
      alive.current = false;
      clearInterval(poll);
      off?.();
    };
  }, [reload]);

  async function act(fn: () => Promise<unknown>) {
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  if (editing) {
    return (
      <div className="min-h-0 flex-1 overflow-y-auto">
        <RuleEditor
          initial={editing.rule}
          isNew={editing.isNew}
          {...{ actionTypes, sounds, media, voices, goals, timers }}
          onCancel={() => setEditing(null)}
          onSave={async (rule) => {
            await api.saveRule(rule);
            setEditing(null);
            await reload();
          }}
        />
      </div>
    );
  }

  return (
    <div className="min-h-0 flex-1 space-y-4 overflow-y-auto">
      <Card
        title={t("rules.title")}
        hint={t("rules.queueStats", { pending: queue.pending, running: queue.running })}
        actions={
          <div className="flex gap-2">
            <Btn onClick={() => void act(async () => { await api.clearQueue(); setQueue(await api.queueStats()); })}>{t("rules.clearQueue")}</Btn>
            <Btn variant="primary" onClick={() => setEditing({ rule: newRule(), isNew: true })}>+ {t("rules.new")}</Btn>
          </div>
        }
      >
        {rules.length === 0 && <p className="py-6 text-center text-sm text-zinc-500">{t("rules.empty")}</p>}
        <ul className="space-y-2">
          {rules.map((r) => (
            <li
              key={r.id}
              className={`flex items-center gap-3 rounded-lg border px-3 py-2 transition-colors ${
                flash[r.id] ? "border-amber-400 bg-amber-400/10" : "border-zinc-800 bg-zinc-950/50"
              }`}
            >
              <input
                type="checkbox"
                checked={r.enabled}
                title={t("rules.enabled")}
                onChange={(e) => void act(async () => { await api.setRuleEnabled(r.id, e.target.checked); await reload(); })}
                className="h-4 w-4 accent-amber-400"
              />
              <div className="min-w-0 flex-1">
                <div className={`truncate text-sm font-semibold ${r.enabled ? "" : "text-zinc-500"}`}>{r.name}</div>
                <div className="truncate text-xs text-zinc-500">
                  {t(`trigger.${r.trigger.type}`)}
                  {triggerSummary(r.trigger) && ` · ${triggerSummary(r.trigger)}`} → {r.plan.steps.map((s) => t(`actionType.${s.action.type}`, { defaultValue: s.action.type })).join(" + ")}
                </div>
              </div>
              <Btn
                onClick={() =>
                  void act(async () => {
                    const outcome = await api.testRule(r.id);
                    setNote(t(`rules.tested.${outcome}`, { name: r.name }));
                    setTimeout(() => setNote(null), 3000);
                  })
                }
              >
                ▶ {t("rules.test")}
              </Btn>
              <Btn onClick={() => setEditing({ rule: structuredClone(r), isNew: false })}>{t("rules.edit")}</Btn>
              <Btn
                onClick={() =>
                  void act(async () => {
                    await api.saveRule({ ...structuredClone(r), id: crypto.randomUUID(), name: `${r.name} (${t("rules.copy")})` });
                    await reload();
                  })
                }
              >
                {t("rules.duplicate")}
              </Btn>
              <Btn
                variant="danger"
                onClick={() => {
                  if (window.confirm(t("rules.confirmDelete", { name: r.name }))) void act(async () => { await api.deleteRule(r.id); await reload(); });
                }}
              >
                {t("rules.delete")}
              </Btn>
            </li>
          ))}
        </ul>
        {note && <p className="mt-3 text-xs text-emerald-400">{note}</p>}
        <div className="mt-2">
          <ErrorText error={error} />
        </div>
      </Card>
    </div>
  );
}

function omitKey(obj: Record<string, number>, key: string): Record<string, number> {
  const { [key]: _removed, ...rest } = obj;
  return rest;
}
