import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { pickFile } from "../../lib/api";
import {
  interactApi as api,
  type BotConfig,
  type BotLogEntry,
  type PointsConfig,
  type PollView,
  type Viewer,
  type WheelConfig,
} from "../../lib/interact";
import { Btn, Card, Checkbox, ErrorText, Field, NumberInput, TextInput, useAction } from "../ui";

const SECTIONS = ["bot", "points", "wheel", "poll", "login"] as const;
type Section = (typeof SECTIONS)[number];

export function InteractPage() {
  const { t } = useTranslation();
  const [section, setSection] = useState<Section>("bot");
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <nav className="flex gap-1 text-sm">
        {SECTIONS.map((s) => (
          <button
            key={s}
            onClick={() => setSection(s)}
            className={`rounded-md px-3 py-1 ${section === s ? "bg-zinc-700 font-semibold" : "text-zinc-400 hover:text-zinc-200"}`}
          >
            {t(`interact.sections.${s}`)}
          </button>
        ))}
      </nav>
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto pb-4">
        {section === "bot" && <BotSection />}
        {section === "points" && <PointsSection />}
        {section === "wheel" && <WheelSection />}
        {section === "poll" && <PollSection />}
        {section === "login" && <LoginSection />}
      </div>
    </div>
  );
}

const uid = () => Math.random().toString(36).slice(2, 10);

function BotSection() {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<BotConfig | null>(null);
  const [log, setLog] = useState<BotLogEntry[]>([]);
  const [saved, setSaved] = useState(false);
  const { run, error, busy } = useAction();

  const refreshLog = useCallback(() => void api.getBotLog().then(setLog).catch(() => undefined), []);
  useEffect(() => {
    void api.getBotConfig().then(setCfg);
    refreshLog();
    const timer = setInterval(refreshLog, 3000);
    return () => clearInterval(timer);
  }, [refreshLog]);

  if (!cfg) return null;
  const update = (patch: Partial<BotConfig>) => {
    setSaved(false);
    setCfg({ ...cfg, ...patch });
  };
  const thanksKeys = ["gift", "follow", "share", "subscribe"] as const;

  return (
    <>
      <Card
        title={t("interact.bot.title")}
        hint={t("interact.bot.hint")}
        actions={
          <Btn
            variant="primary"
            disabled={busy}
            onClick={() => void run(async () => { setCfg(await api.setBotConfig(cfg)); setSaved(true); })}
          >
            {saved ? t("interact.saved") : t("interact.save")}
          </Btn>
        }
      >
        <div className="space-y-3">
          <Checkbox checked={cfg.enabled} onChange={(v) => update({ enabled: v })} label={t("interact.bot.enabled")} />
          <Field label={t("interact.bot.minInterval")}>
            <NumberInput value={cfg.minIntervalMs} min={1000} max={600000} step={500} onChange={(v) => update({ minIntervalMs: v ?? 2000 })} />
          </Field>
          <ErrorText error={error} />
        </div>
      </Card>

      <Card
        title={t("interact.bot.commands")}
        actions={
          <Btn
            onClick={() =>
              update({
                commands: [
                  ...cfg.commands,
                  {
                    id: uid(),
                    enabled: true,
                    names: [""],
                    responses: [""],
                    conditions: { globalCooldownMs: 5000, userCooldownMs: 15000, rolesAny: [], probability: 100 },
                  },
                ],
              })
            }
          >
            {t("interact.add")}
          </Btn>
        }
      >
        <div className="space-y-3">
          {cfg.commands.length === 0 && <p className="text-xs text-zinc-500">{t("interact.bot.noCommands")}</p>}
          {cfg.commands.map((c, i) => {
            const set = (patch: Partial<typeof c>) => update({ commands: cfg.commands.map((x, j) => (j === i ? { ...x, ...patch } : x)) });
            return (
              <div key={c.id} className="grid grid-cols-[1fr_2fr_auto] items-end gap-2">
                <Field label={t("interact.bot.names")}>
                  <TextInput value={c.names.join(", ")} placeholder="discord, ds" onChange={(v) => set({ names: v.split(",").map((s) => s.trim()) })} />
                </Field>
                <Field label={t("interact.bot.response")}>
                  <TextInput value={c.responses[0] ?? ""} placeholder="{user}: discord.gg/…" onChange={(v) => set({ responses: [v] })} />
                </Field>
                <div className="flex items-center gap-2">
                  <Checkbox checked={c.enabled} onChange={(v) => set({ enabled: v })} label="" />
                  <Btn variant="danger" onClick={() => update({ commands: cfg.commands.filter((_, j) => j !== i) })}>
                    ✕
                  </Btn>
                </div>
              </div>
            );
          })}
        </div>
      </Card>

      <Card
        title={t("interact.bot.timed")}
        actions={
          <Btn
            onClick={() => update({ timedMessages: [...cfg.timedMessages, { id: uid(), enabled: true, text: "", everyMinutes: 10, minChatMessages: 3 }] })}
          >
            {t("interact.add")}
          </Btn>
        }
      >
        <div className="space-y-3">
          {cfg.timedMessages.map((m, i) => {
            const set = (patch: Partial<typeof m>) => update({ timedMessages: cfg.timedMessages.map((x, j) => (j === i ? { ...x, ...patch } : x)) });
            return (
              <div key={m.id} className="grid grid-cols-[3fr_1fr_1fr_auto] items-end gap-2">
                <Field label={t("interact.bot.text")}>
                  <TextInput value={m.text} onChange={(v) => set({ text: v })} />
                </Field>
                <Field label={t("interact.bot.everyMinutes")}>
                  <NumberInput value={m.everyMinutes} min={1} max={720} onChange={(v) => set({ everyMinutes: v ?? 10 })} />
                </Field>
                <Field label={t("interact.bot.minChat")}>
                  <NumberInput value={m.minChatMessages} min={0} onChange={(v) => set({ minChatMessages: v ?? 0 })} />
                </Field>
                <div className="flex items-center gap-2">
                  <Checkbox checked={m.enabled} onChange={(v) => set({ enabled: v })} label="" />
                  <Btn variant="danger" onClick={() => update({ timedMessages: cfg.timedMessages.filter((_, j) => j !== i) })}>
                    ✕
                  </Btn>
                </div>
              </div>
            );
          })}
        </div>
      </Card>

      <Card title={t("interact.bot.thanks")} hint={t("interact.bot.thanksHint")}>
        <div className="space-y-3">
          {thanksKeys.map((k) => (
            <div key={k} className="grid grid-cols-[auto_1fr] items-end gap-3">
              <Checkbox
                checked={cfg.thanks[k].enabled}
                onChange={(v) => update({ thanks: { ...cfg.thanks, [k]: { ...cfg.thanks[k], enabled: v } } })}
                label={t(`interact.bot.thanksKinds.${k}`)}
              />
              <TextInput value={cfg.thanks[k].template} onChange={(v) => update({ thanks: { ...cfg.thanks, [k]: { ...cfg.thanks[k], template: v } } })} />
            </div>
          ))}
        </div>
      </Card>

      <Card
        title={t("interact.bot.log")}
        actions={<Btn onClick={() => void api.clearBotLog().then(refreshLog)}>{t("interact.bot.clearLog")}</Btn>}
      >
        <BotSay />
        <ul className="mt-3 max-h-56 space-y-1 overflow-y-auto text-xs">
          {log.length === 0 && <li className="text-zinc-500">{t("interact.bot.emptyLog")}</li>}
          {log.map((e, i) => (
            <li key={`${e.ts}-${i}`} className="flex gap-2">
              <span className={e.status.state === "sent" ? "text-emerald-400" : e.status.state === "failed" ? "text-rose-400" : "text-amber-400"}>
                {t(`interact.bot.state.${e.status.state}`)}
              </span>
              <span className="text-zinc-500">{e.source}</span>
              <span className="min-w-0 flex-1 truncate">{e.text}</span>
              {e.status.reason && <span className="text-zinc-500">{e.status.reason}</span>}
            </li>
          ))}
        </ul>
      </Card>
    </>
  );
}

function BotSay() {
  const { t } = useTranslation();
  const [text, setText] = useState("");
  const { run, error } = useAction();
  return (
    <div className="flex items-end gap-2">
      <Field label={t("interact.bot.testMessage")} className="flex-1">
        <TextInput value={text} onChange={setText} />
      </Field>
      <Btn disabled={!text.trim()} onClick={() => void run(async () => { await api.botSay(text); setText(""); })}>
        {t("interact.bot.send")}
      </Btn>
      <ErrorText error={error} />
    </div>
  );
}

function PointsSection() {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<PointsConfig | null>(null);
  const [rows, setRows] = useState<Viewer[]>([]);
  const [total, setTotal] = useState(0);
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState("points");
  const [saved, setSaved] = useState(false);
  const { run, error, busy } = useAction();

  const load = useCallback(
    () => void api.listViewers(search, sort, 100, 0).then((p) => { setRows(p.items); setTotal(p.total); }).catch(() => undefined),
    [search, sort],
  );
  useEffect(() => { void api.getPointsConfig().then(setCfg); }, []);
  useEffect(load, [load]);

  if (!cfg) return null;
  const upd = (patch: Partial<PointsConfig>) => { setSaved(false); setCfg({ ...cfg, ...patch }); };
  const num = (k: keyof PointsConfig, label: string) => (
    <Field label={label}>
      <NumberInput value={cfg[k] as number} min={0} onChange={(v) => upd({ [k]: v ?? 0 } as Partial<PointsConfig>)} />
    </Field>
  );

  return (
    <>
      <Card
        title={t("interact.points.title")}
        hint={t("interact.points.hint")}
        actions={
          <Btn variant="primary" disabled={busy} onClick={() => void run(async () => { setCfg(await api.setPointsConfig(cfg)); setSaved(true); })}>
            {saved ? t("interact.saved") : t("interact.save")}
          </Btn>
        }
      >
        <div className="space-y-3">
          <Checkbox checked={cfg.enabled} onChange={(v) => upd({ enabled: v })} label={t("interact.points.enabled")} />
          <div className="grid grid-cols-4 gap-3">
            <Field label={t("interact.points.currency")}>
              <TextInput value={cfg.currencyName} onChange={(v) => upd({ currencyName: v })} />
            </Field>
            {num("watchPoints", t("interact.points.watch"))}
            {num("watchIntervalMinutes", t("interact.points.watchEvery"))}
            {num("commentPoints", t("interact.points.comment"))}
            {num("likePoints", t("interact.points.like"))}
            {num("likeEvery", t("interact.points.likeEvery"))}
            {num("sharePoints", t("interact.points.share"))}
            {num("followPoints", t("interact.points.follow"))}
            {num("subscribePoints", t("interact.points.subscribe"))}
            {num("pointsPerCoin", t("interact.points.perCoin"))}
            {num("subscriberMultiplier", t("interact.points.subMultiplier"))}
            {num("topSize", t("interact.points.topSize"))}
            <Field label={t("interact.points.pointsCommand")}>
              <TextInput value={cfg.pointsCommand} onChange={(v) => upd({ pointsCommand: v })} />
            </Field>
            <Field label={t("interact.points.topCommand")}>
              <TextInput value={cfg.topCommand} onChange={(v) => upd({ topCommand: v })} />
            </Field>
          </div>
          <p className="text-xs text-zinc-500">{t("interact.points.rewardsHint")}</p>
          <ErrorText error={error} />
        </div>
      </Card>

      <Card
        title={`${t("interact.points.viewers")} (${total})`}
        actions={
          <div className="flex gap-2">
            <Btn onClick={() => void run(async () => { await api.exportViewersCsv(); })}>{t("interact.points.export")}</Btn>
            <Btn
              onClick={() =>
                void run(async () => {
                  const path = await pickFile("CSV", ["csv"]);
                  if (path) { await api.importViewersCsv(path, "replace"); load(); }
                })
              }
            >
              {t("interact.points.import")}
            </Btn>
          </div>
        }
      >
        <div className="mb-3 flex gap-2">
          <TextInput value={search} placeholder={t("interact.points.search")} onChange={setSearch} />
          <select value={sort} onChange={(e) => setSort(e.target.value)} className="rounded-md border border-zinc-700 bg-zinc-950 px-2 text-sm">
            {["points", "name", "lastSeen", "coinsGifted"].map((s) => (
              <option key={s} value={s}>{t(`interact.points.sort.${s}`)}</option>
            ))}
          </select>
        </div>
        <table className="w-full text-left text-xs">
          <thead className="text-zinc-500">
            <tr>
              <th>{t("interact.points.user")}</th>
              <th>{cfg.currencyName}</th>
              <th>{t("interact.points.coins")}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.map((v) => (
              <tr key={v.userId} className="border-t border-zinc-800">
                <td className="py-1">{v.nickname || v.uniqueId} <span className="text-zinc-500">@{v.uniqueId}</span></td>
                <td>{v.points}</td>
                <td>{v.coinsGifted}</td>
                <td className="flex justify-end gap-1 py-1">
                  <Btn onClick={() => void run(async () => { await api.adjustViewerPoints(v.userId, 100); load(); })}>+100</Btn>
                  <Btn onClick={() => void run(async () => { await api.adjustViewerPoints(v.userId, -100); load(); })}>−100</Btn>
                  <Btn variant="danger" onClick={() => void run(async () => { await api.deleteViewer(v.userId); load(); })}>✕</Btn>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>
    </>
  );
}

function WheelSection() {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<WheelConfig | null>(null);
  const [saved, setSaved] = useState(false);
  const { run, error, busy } = useAction();
  useEffect(() => { void api.getWheelConfig().then(setCfg); }, []);
  if (!cfg) return null;
  const upd = (patch: Partial<WheelConfig>) => { setSaved(false); setCfg({ ...cfg, ...patch }); };

  return (
    <Card
      title={t("interact.wheel.title")}
      hint={t("interact.wheel.hint")}
      actions={
        <div className="flex gap-2">
          <Btn disabled={busy} onClick={() => void run(() => api.spinWheelTest())}>{t("interact.wheel.test")}</Btn>
          <Btn variant="primary" disabled={busy} onClick={() => void run(async () => { setCfg(await api.setWheelConfig(cfg)); setSaved(true); })}>
            {saved ? t("interact.saved") : t("interact.save")}
          </Btn>
        </div>
      }
    >
      <div className="space-y-3">
        <Field label={t("interact.wheel.announce")}>
          <TextInput value={cfg.announce} onChange={(v) => upd({ announce: v })} />
        </Field>
        {cfg.segments.map((s, i) => {
          const set = (patch: Partial<typeof s>) => upd({ segments: cfg.segments.map((x, j) => (j === i ? { ...x, ...patch } : x)) });
          return (
            <div key={s.id} className="grid grid-cols-[3fr_1fr_auto_auto] items-end gap-2">
              <Field label={t("interact.wheel.prize")}>
                <TextInput value={s.label} onChange={(v) => set({ label: v })} />
              </Field>
              <Field label={t("interact.wheel.weight")}>
                <NumberInput value={s.weight} min={0} onChange={(v) => set({ weight: v ?? 0 })} />
              </Field>
              <input type="color" value={s.color} onChange={(e) => set({ color: e.target.value })} className="h-8 w-10 bg-transparent" />
              <Btn variant="danger" onClick={() => upd({ segments: cfg.segments.filter((_, j) => j !== i) })}>✕</Btn>
            </div>
          );
        })}
        <Btn onClick={() => upd({ segments: [...cfg.segments, { id: uid(), label: "", weight: 1, color: "#f59e0b", plan: { mode: "sequence", steps: [] } }] })}>
          {t("interact.add")}
        </Btn>
        <p className="text-xs text-zinc-500">{t("interact.wheel.actionHint")}</p>
        <ErrorText error={error} />
      </div>
    </Card>
  );
}

function PollSection() {
  const { t } = useTranslation();
  const [question, setQuestion] = useState("");
  const [options, setOptions] = useState("");
  const [seconds, setSeconds] = useState<number | null>(60);
  const [poll, setPoll] = useState<PollView | null>(null);
  const { run, error, busy } = useAction();
  useEffect(() => {
    const load = () => void api.getPoll().then(setPoll).catch(() => undefined);
    load();
    const timer = setInterval(load, 1000);
    return () => clearInterval(timer);
  }, []);

  return (
    <Card title={t("interact.poll.title")} hint={t("interact.poll.hint")}>
      <div className="space-y-3">
        <Field label={t("interact.poll.question")}>
          <TextInput value={question} onChange={setQuestion} />
        </Field>
        <Field label={t("interact.poll.options")} hint={t("interact.poll.optionsHint")}>
          <textarea
            value={options}
            onChange={(e) => setOptions(e.target.value)}
            rows={4}
            className="w-full rounded-md border border-zinc-700 bg-zinc-950 px-2 py-1.5 text-sm outline-none focus:border-amber-400"
          />
        </Field>
        <Field label={t("interact.poll.duration")}>
          <NumberInput value={seconds} min={5} max={3600} onChange={setSeconds} />
        </Field>
        <div className="flex gap-2">
          <Btn
            variant="primary"
            disabled={busy}
            onClick={() => void run(async () => setPoll(await api.startPoll(question, options.split("\n"), seconds ?? 60)))}
          >
            {t("interact.poll.start")}
          </Btn>
          <Btn disabled={!poll || poll.ended} onClick={() => void run(async () => { await api.stopPoll(); })}>{t("interact.poll.stop")}</Btn>
          <Btn onClick={() => void run(async () => { await api.clearPoll(); setPoll(null); })}>{t("interact.poll.clear")}</Btn>
        </div>
        <ErrorText error={error} />
        {poll && (
          <ul className="space-y-1 text-sm">
            <li className="font-semibold">{poll.question} {poll.ended && `· ${t("interact.poll.ended")}`}</li>
            {poll.options.map((o, i) => (
              <li key={i} className={poll.winners.includes(i) ? "text-amber-400" : ""}>
                {i + 1}. {o.label} — {o.votes}
              </li>
            ))}
          </ul>
        )}
      </div>
    </Card>
  );
}

function LoginSection() {
  const { t } = useTranslation();
  const [has, setHas] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const { run, error, busy } = useAction();
  const refresh = () => void api.hasTiktokSession().then(setHas);
  useEffect(refresh, []);
  return (
    <Card title={t("interact.login.title")} hint={t("interact.login.hint")}>
      <div className="space-y-3">
        <p className="text-sm">{has ? t("interact.login.active") : t("interact.login.none")}</p>
        <div className="flex flex-wrap gap-2">
          <Btn variant="primary" disabled={busy} onClick={() => void run(async () => { await api.tiktokLoginStart(); setMsg(t("interact.login.opened")); })}>
            {t("interact.login.start")}
          </Btn>
          <Btn
            disabled={busy}
            onClick={() =>
              void run(async () => {
                const ok = await api.tiktokLoginFinish();
                setMsg(ok ? t("interact.login.saved") : t("interact.login.notYet"));
                refresh();
              })
            }
          >
            {t("interact.login.finish")}
          </Btn>
          {has && (
            <Btn variant="danger" disabled={busy} onClick={() => void run(async () => { await api.tiktokLogout(); setMsg(null); refresh(); })}>
              {t("interact.login.logout")}
            </Btn>
          )}
        </div>
        {msg && <p className="text-xs text-zinc-400">{msg}</p>}
        <p className="text-xs text-zinc-500">{t("interact.login.reconnect")}</p>
        <ErrorText error={error} />
      </div>
    </Card>
  );
}
