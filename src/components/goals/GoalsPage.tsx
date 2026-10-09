import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../../lib/api";
import type { Goal, TimerConfig, TimerView } from "../../lib/types";
import { Btn, Card, ErrorText, NumberInput } from "../ui";
import { GoalEditor, newGoal } from "./GoalEditor";
import { newTimer, TimerEditor } from "./TimerEditor";

const POLL_MS = 1000;

function clock(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const p = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${p(h)}:${p(m)}:${p(s)}` : `${p(m)}:${p(s)}`;
}

export function GoalsPage() {
  const { t } = useTranslation();
  const [goals, setGoals] = useState<Goal[]>([]);
  const [timers, setTimers] = useState<TimerView[]>([]);
  const [editGoal, setEditGoal] = useState<{ goal: Goal; isNew: boolean } | null>(null);
  const [editTimer, setEditTimer] = useState<{ cfg: TimerConfig; isNew: boolean } | null>(null);
  const [step, setStep] = useState<number | null>(100);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);

  const reload = useCallback(async () => {
    try {
      const [g, tm] = await Promise.all([api.listGoals(), api.listTimers()]);
      if (!alive.current) return;
      setGoals(g);
      setTimers(tm);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    void reload();
    const poll = setInterval(() => void reload(), POLL_MS);
    return () => {
      alive.current = false;
      clearInterval(poll);
    };
  }, [reload]);

  async function act(fn: () => Promise<unknown>) {
    setError(null);
    try {
      await fn();
      await reload();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  if (editGoal) {
    return (
      <div className="min-h-0 flex-1 overflow-y-auto">
        <GoalEditor
          initial={editGoal.goal}
          isNew={editGoal.isNew}
          onCancel={() => setEditGoal(null)}
          onSave={async (g) => {
            await api.saveGoal(g);
            setEditGoal(null);
            await reload();
          }}
        />
      </div>
    );
  }
  if (editTimer) {
    return (
      <div className="min-h-0 flex-1 overflow-y-auto">
        <TimerEditor
          initial={editTimer.cfg}
          isNew={editTimer.isNew}
          onCancel={() => setEditTimer(null)}
          onSave={async (c) => {
            await api.saveTimer(c);
            setEditTimer(null);
            await reload();
          }}
        />
      </div>
    );
  }

  const delta = step ?? 0;

  return (
    <div className="min-h-0 flex-1 space-y-4 overflow-y-auto">
      <Card
        title={t("goals.title")}
        hint={t("goals.hint")}
        actions={<Btn variant="primary" onClick={() => setEditGoal({ goal: newGoal(), isNew: true })}>+ {t("goals.new")}</Btn>}
      >
        {goals.length === 0 && <p className="py-4 text-center text-sm text-zinc-500">{t("goals.empty")}</p>}
        <ul className="space-y-3">
          {goals.map((g) => (
            <li key={g.id} className="rounded-lg border border-zinc-800 bg-zinc-950/50 p-3">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="truncate text-sm font-semibold">{g.name}</div>
                  <div className="text-xs text-zinc-500">
                    {t(`goals.kinds.${g.kind.type}`)}
                    {g.kind.type === "gift" && g.kind.giftName ? ` · ${g.kind.giftName}` : ""} · {t("goals.reached", { n: g.reachedCount })}
                  </div>
                </div>
                <div className="flex shrink-0 gap-2">
                  <Btn onClick={() => void act(() => api.adjustGoal(g.id, delta))} disabled={delta === 0}>+{delta}</Btn>
                  <Btn onClick={() => void act(() => api.adjustGoal(g.id, -delta))} disabled={delta === 0}>−{delta}</Btn>
                  <Btn onClick={() => void act(() => api.resetGoal(g.id))}>{t("goals.resetProgress")}</Btn>
                  <Btn onClick={() => setEditGoal({ goal: structuredClone(g), isNew: false })}>{t("rules.edit")}</Btn>
                  <Btn
                    variant="danger"
                    onClick={() => window.confirm(t("goals.confirmDelete", { name: g.name })) && void act(() => api.deleteGoal(g.id))}
                  >
                    {t("rules.delete")}
                  </Btn>
                </div>
              </div>
              <div className="mt-2 h-3 overflow-hidden rounded-full bg-zinc-800">
                <div className="h-full bg-amber-400 transition-all" style={{ width: `${Math.min(100, (g.current / Math.max(1, g.target)) * 100)}%` }} />
              </div>
              <div className="mt-1 text-right text-xs text-zinc-400">
                {g.current.toLocaleString()} / {g.target.toLocaleString()}
              </div>
            </li>
          ))}
        </ul>
        <div className="mt-3 flex items-center gap-2 text-xs text-zinc-400">
          <span>{t("goals.adjustBy")}</span>
          <div className="w-28">
            <NumberInput value={step} min={1} onChange={setStep} />
          </div>
        </div>
      </Card>

      <Card
        title={t("timers.title")}
        hint={t("timers.hint")}
        actions={<Btn variant="primary" onClick={() => setEditTimer({ cfg: newTimer(), isNew: true })}>+ {t("timers.new")}</Btn>}
      >
        {timers.length === 0 && <p className="py-4 text-center text-sm text-zinc-500">{t("timers.empty")}</p>}
        <ul className="space-y-3">
          {timers.map((tm) => {
            const id = tm.config.id;
            return (
              <li key={id} className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-zinc-800 bg-zinc-950/50 p-3">
                <div>
                  <div className="text-sm font-semibold">{tm.config.name}</div>
                  <div className="text-xs text-zinc-500">{t(`timers.status.${tm.status}`)}</div>
                </div>
                <div className={`font-mono text-3xl font-bold tabular-nums ${tm.status === "running" ? "text-amber-300" : "text-zinc-400"}`}>{clock(tm.remainingMs)}</div>
                <div className="flex flex-wrap gap-2">
                  {tm.status === "running" ? (
                    <Btn onClick={() => void act(() => api.controlTimer(id, "pause"))}>⏸ {t("timers.pause")}</Btn>
                  ) : (
                    <Btn variant="primary" onClick={() => void act(() => api.controlTimer(id, tm.status === "paused" ? "resume" : "start"))}>▶ {tm.status === "paused" ? t("timers.resume") : t("timers.start")}</Btn>
                  )}
                  <Btn onClick={() => void act(() => api.controlTimer(id, "add", 60))}>+1 min</Btn>
                  <Btn onClick={() => void act(() => api.controlTimer(id, "add", -60))}>−1 min</Btn>
                  <Btn onClick={() => void act(() => api.controlTimer(id, "reset"))}>{t("timers.reset")}</Btn>
                  <Btn onClick={() => setEditTimer({ cfg: structuredClone(tm.config), isNew: false })}>{t("rules.edit")}</Btn>
                  <Btn variant="danger" onClick={() => window.confirm(t("goals.confirmDelete", { name: tm.config.name })) && void act(() => api.deleteTimer(id))}>
                    {t("rules.delete")}
                  </Btn>
                </div>
              </li>
            );
          })}
        </ul>
      </Card>

      <Card title={t("session.title")} hint={t("session.hint")}>
        <div className="flex gap-2">
          <Btn onClick={() => window.confirm(t("session.confirmNew")) && void act(() => api.newSession())}>{t("session.new")}</Btn>
          <Btn variant="danger" onClick={() => window.confirm(t("session.confirmClear")) && void act(() => api.clearDonorHistory())}>
            {t("session.clearHistory")}
          </Btn>
        </div>
      </Card>
      <ErrorText error={error} />
    </div>
  );
}
