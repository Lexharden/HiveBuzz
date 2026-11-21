import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../../lib/api";
import { filterGifts, invalidateGiftCatalog } from "../../lib/gifts";
import type { GiftsInfo, GiftView, ManualGift } from "../../lib/types";
import { Btn, Card, ErrorText, Field, NumberInput, TextInput } from "../ui";
import { GiftThumb, useGiftCatalog } from "./GiftPicker";

/** Filas visibles a la vez (el catálogo entero son unos cientos: se busca para encontrar el resto). */
const PAGE = 60;

interface Draft {
  previousKey?: string;
  id: number | null;
  nameEn: string;
  nameEs: string;
  diamonds: number | null;
}

const draftOf = (g: GiftView): Draft => ({
  // Uno solo de la lista oficial aún no tiene fila propia: guardarlo la crea con su id.
  previousKey: g.referenceOnly ? undefined : g.key,
  id: g.id,
  nameEn: g.nameEn ?? "",
  nameEs: g.nameEs ?? "",
  diamonds: g.diamonds,
});

const EMPTY: Draft = { id: null, nameEn: "", nameEs: "", diamonds: null };

/** Catálogo de regalos: la lista oficial incluida en la app, lo aprendido en directo y lo corregido a mano. */
export function GiftCatalogCard() {
  const { t } = useTranslation();
  const cat = useGiftCatalog();
  const [info, setInfo] = useState<GiftsInfo | null>(null);
  const [query, setQuery] = useState("");
  const [shown, setShown] = useState(PAGE);
  const [editing, setEditing] = useState<{ draft: Draft; gift?: GiftView } | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reloadInfo = useCallback(() => {
    api.giftsInfo().then(setInfo, (e: unknown) => setError(errorMessage(e)));
  }, []);
  useEffect(reloadInfo, [reloadInfo, cat]);

  const matches = cat ? filterGifts(cat.gifts, query) : [];
  const visible = matches.slice(0, shown);

  async function act(fn: () => Promise<unknown>) {
    setError(null);
    try {
      await fn();
      setEditing(null);
      invalidateGiftCatalog();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  const save = (d: Draft) => {
    const gift: ManualGift = {
      ...(d.previousKey ? { previousKey: d.previousKey } : {}),
      ...(d.id !== null ? { id: d.id } : {}),
      ...(d.nameEn.trim() ? { nameEn: d.nameEn } : {}),
      ...(d.nameEs.trim() ? { nameEs: d.nameEs } : {}),
      ...(d.diamonds !== null ? { diamonds: d.diamonds } : {}),
    };
    return act(() => api.saveGift(gift));
  };

  return (
    <Card
      title={t("gifts.title")}
      hint={t("gifts.hint")}
      actions={<Btn onClick={() => setEditing({ draft: EMPTY })}>+ {t("gifts.add")}</Btn>}
    >
      {info && <p className="mb-2 text-xs text-zinc-500">{t("gifts.stats", { ...info })}</p>}
      <TextInput
        value={query}
        onChange={(v) => {
          setQuery(v);
          setShown(PAGE);
        }}
        placeholder={t("gifts.search")}
      />
      {editing && !editing.gift && <Editor draft={editing.draft} onSave={save} onCancel={() => setEditing(null)} />}
      <ErrorText error={error} />
      {!cat && <p className="mt-2 text-xs text-zinc-500">{t("gifts.loading")}</p>}
      {cat && matches.length === 0 && <p className="mt-2 text-xs text-zinc-500">{t("gifts.none")}</p>}
      <ul className="mt-2 divide-y divide-zinc-800">
        {visible.map((g) => (
          <li key={g.key} className="py-1.5">
            <div className="flex items-center gap-2 text-sm">
              <GiftThumb gift={g} cat={cat} size={32} />
              <div className="min-w-0 flex-1">
                <div className="truncate text-zinc-200">{g.displayName}</div>
                <div className="truncate text-xs text-zinc-500">
                  {[g.id !== null ? `#${[g.id, ...g.otherIds].join(" · #")}` : null, g.also.length ? t("gifts.aliases", { list: g.also.join(", ") }) : null]
                    .filter(Boolean)
                    .join(" · ")}
                </div>
              </div>
              {g.diamonds !== null && <span className="shrink-0 text-xs text-zinc-300">{g.diamonds} 💎</span>}
              <span className="w-24 shrink-0 text-right text-xs text-zinc-500">
                {g.manual ? t("gifts.badge.manual") : g.seen > 0 ? t("gifts.badge.seen", { count: g.seen }) : t("gifts.badge.official")}
              </span>
              <Btn onClick={() => setEditing(editing?.gift?.key === g.key ? null : { draft: draftOf(g), gift: g })}>{t("gifts.edit")}</Btn>
            </div>
            {editing?.gift?.key === g.key && (
              <Editor
                draft={editing.draft}
                onSave={save}
                onCancel={() => setEditing(null)}
                onReset={g.referenceOnly ? undefined : () => window.confirm(t("gifts.confirmReset", { name: g.displayName })) && act(() => api.deleteGift(g.key))}
              />
            )}
          </li>
        ))}
      </ul>
      {matches.length > shown && (
        <div className="mt-2 flex items-center justify-between text-xs text-zinc-500">
          <span>{t("gifts.showing", { shown, total: matches.length })}</span>
          <Btn onClick={() => setShown((n) => n + PAGE)}>{t("gifts.more")}</Btn>
        </div>
      )}
    </Card>
  );
}

function Editor({ draft, onSave, onCancel, onReset }: { draft: Draft; onSave: (d: Draft) => Promise<unknown>; onCancel: () => void; onReset?: () => unknown }) {
  const { t } = useTranslation();
  const [d, setD] = useState(draft);
  const patch = (p: Partial<Draft>) => setD((x) => ({ ...x, ...p }));
  return (
    <div className="mt-2 space-y-2 rounded-md border border-zinc-800 bg-zinc-950/60 p-3">
      <div className="grid grid-cols-2 gap-2">
        <Field label={t("gifts.nameEs")} hint={t("gifts.nameEsHint")}>
          <TextInput value={d.nameEs} onChange={(nameEs) => patch({ nameEs })} />
        </Field>
        <Field label={t("gifts.nameEn")} hint={t("gifts.nameEnHint")}>
          <TextInput value={d.nameEn} onChange={(nameEn) => patch({ nameEn })} />
        </Field>
        <Field label={t("trigger.giftId")}>
          <NumberInput value={d.id} min={1} onChange={(id) => patch({ id })} />
        </Field>
        <Field label={t("gifts.diamonds")}>
          <NumberInput value={d.diamonds} min={0} onChange={(diamonds) => patch({ diamonds })} />
        </Field>
      </div>
      <div className="flex justify-end gap-2">
        {onReset && (
          <Btn variant="danger" className="mr-auto" onClick={onReset}>
            {t("gifts.reset")}
          </Btn>
        )}
        <Btn onClick={onCancel}>{t("gifts.cancel")}</Btn>
        <Btn variant="primary" onClick={() => onSave(d)}>
          {t("gifts.save")}
        </Btn>
      </div>
    </div>
  );
}
