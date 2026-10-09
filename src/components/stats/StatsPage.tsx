import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { systemApi as api, type StreamStats, type StreamSummary } from "../../lib/system";
import { Btn, Card, ErrorText, useAction } from "../ui";

const fmt = (n: number) => new Intl.NumberFormat().format(n);

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-950 p-3">
      <div className="text-xs text-zinc-500">{label}</div>
      <div className="text-xl font-bold text-amber-400">{fmt(value)}</div>
    </div>
  );
}

export function StatsPage() {
  const { t } = useTranslation();
  const [streams, setStreams] = useState<StreamSummary[]>([]);
  const [selected, setSelected] = useState<StreamStats | null>(null);
  const { run, error, busy } = useAction();

  const refresh = useCallback(() => void run(async () => setStreams(await api.listStreams())), [run]);
  useEffect(refresh, [refresh]);

  const open = (id: number) => void run(async () => setSelected(await api.getStream(id)));
  const when = (ms: number) => new Date(ms).toLocaleString();

  return (
    <div className="grid min-h-0 flex-1 grid-cols-[18rem_1fr] gap-4">
      <Card title={t("stats.streams")} hint={t("stats.hint")} actions={<Btn onClick={refresh} disabled={busy}>{t("stats.refresh")}</Btn>}>
        <div className="max-h-[60vh] space-y-1 overflow-y-auto">
          {streams.length === 0 && <p className="text-xs text-zinc-500">{t("stats.empty")}</p>}
          {streams.map((s) => (
            <button
              key={s.id}
              onClick={() => open(s.id)}
              className={`block w-full rounded-md px-2 py-1.5 text-left text-xs ${selected?.id === s.id ? "bg-zinc-700" : "hover:bg-zinc-800"}`}
            >
              <div className="font-semibold">{when(s.id)}</div>
              <div className="text-zinc-400">
                🪙 {fmt(s.coins)} · 👁 {fmt(s.peakViewers)}
              </div>
            </button>
          ))}
        </div>
        <ErrorText error={error} />
      </Card>

      <div className="min-h-0 space-y-4 overflow-y-auto pb-4">
        {!selected && <p className="text-sm text-zinc-500">{t("stats.pick")}</p>}
        {selected && (
          <>
            <Card
              title={when(selected.id)}
              actions={
                <Btn
                  variant="danger"
                  onClick={() =>
                    void run(async () => {
                      await api.deleteStream(selected.id);
                      setSelected(null);
                      setStreams(await api.listStreams());
                    })
                  }
                >
                  {t("stats.delete")}
                </Btn>
              }
            >
              <div className="grid grid-cols-4 gap-2">
                <Stat label={t("stats.coins")} value={selected.coins} />
                <Stat label={t("stats.peak")} value={selected.peakViewers} />
                <Stat label={t("stats.gifts")} value={selected.giftsTotal} />
                <Stat label={t("stats.chats")} value={selected.chats} />
                <Stat label={t("stats.likes")} value={selected.likes} />
                <Stat label={t("stats.follows")} value={selected.follows} />
                <Stat label={t("stats.shares")} value={selected.shares} />
                <Stat label={t("stats.subscribers")} value={selected.subscribers} />
              </div>
            </Card>
            <div className="grid grid-cols-2 gap-4">
              <Card title={t("stats.topDonors")}>
                {selected.donors.length === 0 && <p className="text-xs text-zinc-500">{t("stats.noDonors")}</p>}
                <ol className="space-y-1 text-sm">
                  {selected.donors.slice(0, 15).map((d, i) => (
                    <li key={d.userId} className="flex justify-between gap-2">
                      <span className="truncate">
                        {i + 1}. {d.nickname} <span className="text-zinc-500">@{d.uniqueId}</span>
                      </span>
                      <span className="text-amber-400">🪙 {fmt(d.coins)}</span>
                    </li>
                  ))}
                </ol>
              </Card>
              <Card title={t("stats.giftsByType")}>
                {selected.gifts.length === 0 && <p className="text-xs text-zinc-500">{t("stats.noGifts")}</p>}
                <ul className="space-y-1 text-sm">
                  {selected.gifts.slice(0, 20).map((g) => (
                    <li key={g.name} className="flex justify-between gap-2">
                      <span className="truncate">
                        {g.name} <span className="text-zinc-500">×{fmt(g.count)}</span>
                      </span>
                      <span className="text-amber-400">🪙 {fmt(g.coins)}</span>
                    </li>
                  ))}
                </ul>
              </Card>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
