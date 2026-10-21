import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../../lib/api";
import type { Instant } from "../../lib/types";
import { Btn, Card, ErrorText, TextInput } from "../ui";

/** Buscador de MyInstants: se escucha sin guardar y «Descargar» lo añade a la biblioteca. */
export function MyInstantsCard({ savedNames, onDownloaded }: { savedNames: string[]; onDownloaded: () => Promise<void> }) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [term, setTerm] = useState("");
  const [page, setPage] = useState(1);
  const [items, setItems] = useState<Instant[]>([]);
  const [more, setMore] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<Set<string>>(new Set());
  const [working, setWorking] = useState<string | null>(null);
  const request = useRef(0);

  const load = useCallback(async (q: string, p: number) => {
    const mine = ++request.current;
    setBusy(true);
    setError(null);
    try {
      const found = await api.myinstantsSearch(q, p);
      if (mine !== request.current) return;
      setItems((cur) => (p === 1 ? found : [...cur, ...found.filter((f) => !cur.some((c) => c.slug === f.slug))]));
      setPage(p);
      setMore(found.length >= 30);
    } catch (e) {
      if (mine === request.current) setError(errorMessage(e));
    } finally {
      if (mine === request.current) setBusy(false);
    }
  }, []);

  useEffect(() => {
    void load("", 1);
  }, [load]);

  const search = () => {
    setTerm(query.trim());
    void load(query.trim(), 1);
  };
  const guard = (fn: () => Promise<unknown>) => void fn().catch((e: unknown) => setError(errorMessage(e)));

  return (
    <Card title={t("myinstants.title")} hint={t("myinstants.hint")}>
      <div className="mb-3 flex gap-2">
        <TextInput
          value={query}
          onChange={setQuery}
          placeholder={t("myinstants.placeholder")}
          onKeyDown={(e) => e.key === "Enter" && search()}
          className="flex-1"
        />
        <Btn variant="primary" disabled={busy} onClick={search}>
          {t("myinstants.search")}
        </Btn>
      </div>
      <p className="mb-2 text-xs text-zinc-500">{term ? t("myinstants.resultsFor", { term }) : t("myinstants.trending")}</p>
      {!busy && items.length === 0 && !error && <p className="py-4 text-center text-sm text-zinc-500">{t("myinstants.none")}</p>}
      <ul className="space-y-2">
        {items.map((i) => {
          const isSaved = saved.has(i.slug) || savedNames.includes(i.name);
          return (
            <li key={i.slug} className="flex items-center gap-3 rounded-lg border border-zinc-800 bg-zinc-950/50 px-3 py-2">
              <Btn onClick={() => api.myinstantsPreview(i.file).catch((e: unknown) => setError(errorMessage(e)))} title={t("myinstants.listen")}>▶</Btn>
              <span className="min-w-0 flex-1 truncate text-sm" title={i.name}>{i.name}</span>
              <Btn
                variant={isSaved ? "ghost" : "primary"}
                disabled={isSaved || working === i.slug}
                onClick={() =>
                  guard(async () => {
                    setWorking(i.slug);
                    try {
                      await api.myinstantsDownload(i.file, i.name);
                      setSaved((cur) => new Set(cur).add(i.slug));
                      await onDownloaded();
                    } finally {
                      setWorking(null);
                    }
                  })
                }
              >
                {isSaved ? t("myinstants.saved") : working === i.slug ? t("myinstants.downloading") : `⬇ ${t("myinstants.download")}`}
              </Btn>
            </li>
          );
        })}
      </ul>
      {more && (
        <div className="mt-3 text-center">
          <Btn disabled={busy} onClick={() => load(term, page + 1)}>
            {busy ? t("myinstants.loading") : t("myinstants.more")}
          </Btn>
        </div>
      )}
      <ErrorText error={error} />
    </Card>
  );
}
