import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { pickFile } from "../../lib/api";
import {
  interactApi as api,
  type PointsConfig,
  type PollView,
  type Viewer,
  type WheelConfig,
} from "../../lib/interact";
import { ViewerGroupsSection } from "../groups/ViewerGroupsSection";
import { Btn, Card, Checkbox, ErrorText, Field, NumberInput, TextInput, useAction } from "../ui";

const SECTIONS = ["points", "groups", "wheel", "poll"] as const;
type Section = (typeof SECTIONS)[number];

export function InteractPage() {
  const { t } = useTranslation();
  const [section, setSection] = useState<Section>("points");
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
        {section === "points" && <PointsSection />}
        {section === "groups" && <ViewerGroupsSection />}
        {section === "wheel" && <WheelSection />}
        {section === "poll" && <PollSection />}
      </div>
    </div>
  );
}

const uid = () => Math.random().toString(36).slice(2, 10);

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
          <Btn variant="primary" disabled={busy} onClick={() => run(async () => { setCfg(await api.setPointsConfig(cfg)); setSaved(true); })}>
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
          </div>
          <p className="text-xs text-zinc-500">{t("interact.points.rewardsHint")}</p>
          <ErrorText error={error} />
        </div>
      </Card>

      <Card
        title={`${t("interact.points.viewers")} (${total})`}
        actions={
          <div className="flex gap-2">
            <Btn onClick={() => run(async () => { await api.exportViewersCsv(); })}>{t("interact.points.export")}</Btn>
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
              <th>{t("interact.points.levels")}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.map((v) => (
              <tr key={v.userId} className="border-t border-zinc-800">
                <td className="py-1">{v.nickname || v.uniqueId} <span className="text-zinc-500">@{v.uniqueId}</span></td>
                <td>{v.points}</td>
                <td>{v.coinsGifted}</td>
                <td className="text-zinc-400">{v.teamLevel > 0 || v.gifterLevel > 0 ? `${v.teamLevel} / ${v.gifterLevel}` : "—"}</td>
                <td className="flex justify-end gap-1 py-1">
                  <Btn onClick={() => run(async () => { await api.adjustViewerPoints(v.userId, 100); load(); })}>+100</Btn>
                  <Btn onClick={() => run(async () => { await api.adjustViewerPoints(v.userId, -100); load(); })}>−100</Btn>
                  <Btn variant="danger" onClick={() => run(async () => { await api.deleteViewer(v.userId); load(); })}>✕</Btn>
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
          <Btn disabled={busy} onClick={() => run(() => api.spinWheelTest())}>{t("interact.wheel.test")}</Btn>
          <Btn variant="primary" disabled={busy} onClick={() => run(async () => { setCfg(await api.setWheelConfig(cfg)); setSaved(true); })}>
            {saved ? t("interact.saved") : t("interact.save")}
          </Btn>
        </div>
      }
    >
      <div className="space-y-3">
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
            onClick={() => run(async () => setPoll(await api.startPoll(question, options.split("\n"), seconds ?? 60)))}
          >
            {t("interact.poll.start")}
          </Btn>
          <Btn disabled={!poll || poll.ended} onClick={() => run(async () => { await api.stopPoll(); })}>{t("interact.poll.stop")}</Btn>
          <Btn onClick={() => run(async () => { await api.clearPoll(); setPoll(null); })}>{t("interact.poll.clear")}</Btn>
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
