import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { filterGifts, findGift, giftImageUrl, giftSummary, loadGiftCatalog, onGiftCatalogChange, parseTyped, primaryName, type GiftCatalog, type GiftSelection } from "../../lib/gifts";
import type { GiftView } from "../../lib/types";
import { inputCls } from "../ui";

/** Resultados que muestra la lista desplegable. */
const MAX_OPTIONS = 40;

/** El catálogo compartido; se refresca solo cuando alguien lo edita. `null` mientras carga (o si falló). */
export function useGiftCatalog(): GiftCatalog | null {
  const [cat, setCat] = useState<GiftCatalog | null>(null);
  useEffect(() => {
    let alive = true;
    const load = () =>
      void loadGiftCatalog().then(
        (c) => alive && setCat(c),
        () => alive && setCat(null),
      );
    load();
    const off = onGiftCatalogChange(load);
    return () => {
      alive = false;
      off();
    };
  }, []);
  return cat;
}

export function GiftThumb({ gift, cat, size = 24 }: { gift: GiftView | undefined; cat: GiftCatalog | null; size?: number }) {
  const src = gift ? giftImageUrl(gift, cat?.base, cat?.token) : undefined;
  const box = { width: size, height: size };
  if (!src) return <span style={box} className="inline-flex shrink-0 items-center justify-center rounded bg-zinc-800 text-[10px] text-zinc-500">🎁</span>;
  return <img src={src} alt="" style={box} className="shrink-0 object-contain" loading="lazy" draggable={false} />;
}

/**
 * Elige un regalo del catálogo buscando por nombre (español o inglés), alias o id. También admite escribir un
 * nombre que aún no está en el catálogo: entonces se compara por nombre, sin mayúsculas ni acentos.
 */
export function GiftPicker({ value, onChange, placeholder }: { value: GiftSelection; onChange: (v: GiftSelection) => void; placeholder?: string }) {
  const { t } = useTranslation();
  const cat = useGiftCatalog();
  const listId = useId();
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const text = value.giftName ?? (value.giftId !== undefined ? `#${value.giftId}` : "");
  const selected = cat ? findGift(cat.gifts, value) : undefined;
  const query = text.replace(/^#/, "");
  const options = open && cat ? filterGifts(cat.gifts, selected && isChosenName(text, selected) ? "" : query, MAX_OPTIONS) : [];

  const choose = (g: GiftView) => {
    onChange({ ...(g.id !== null ? { giftId: g.id } : {}), giftName: primaryName(g) });
    setOpen(false);
  };

  return (
    <div className="relative">
      <div className="flex items-center gap-2">
        <GiftThumb gift={selected} cat={cat} />
        <input
          role="combobox"
          aria-expanded={open}
          aria-controls={listId}
          aria-autocomplete="list"
          value={text}
          placeholder={placeholder ?? t("gifts.picker.placeholder")}
          onFocus={() => setOpen(true)}
          onBlur={() => setOpen(false)}
          onChange={(e) => {
            const v = e.target.value;
            // Otro nombre escrito a mano: el id de la elección anterior ya no vale.
            onChange(parseTyped(v));
            setOpen(true);
            setActive(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown" || e.key === "ArrowUp") {
              e.preventDefault();
              setOpen(true);
              const n = Math.max(options.length, 1);
              setActive((a) => (a + (e.key === "ArrowDown" ? 1 : n - 1)) % n);
            } else if (e.key === "Enter" && open && options[active]) {
              e.preventDefault();
              choose(options[active]);
            } else if (e.key === "Escape") {
              setOpen(false);
            }
          }}
          className={inputCls}
        />
      </div>
      {open && options.length > 0 && (
        <ul id={listId} role="listbox" className="absolute z-30 mt-1 max-h-72 w-full min-w-64 overflow-y-auto rounded-md border border-zinc-700 bg-zinc-900 py-1 shadow-xl">
          {options.map((g, i) => (
            <li
              key={g.key}
              role="option"
              aria-selected={i === active}
              // mousedown y no click: el blur del campo cerraría la lista antes del click.
              onMouseDown={(e) => {
                e.preventDefault();
                choose(g);
              }}
              onMouseEnter={() => setActive(i)}
              className={`flex cursor-pointer items-center gap-2 px-2 py-1 text-xs ${i === active ? "bg-amber-400/15 text-amber-100" : "text-zinc-300"}`}
            >
              <GiftThumb gift={g} cat={cat} />
              <span className="min-w-0 flex-1 truncate">{g.displayName}</span>
              {g.diamonds !== null && <span className="shrink-0 text-zinc-400">{g.diamonds} 💎</span>}
              {g.id !== null && <span className="shrink-0 text-zinc-500">#{g.id}</span>}
            </li>
          ))}
        </ul>
      )}
      <span className="mt-1 block truncate text-zinc-500">
        {selected
          ? `✓ ${giftSummary(selected)}${selected.id !== null ? ` · #${selected.id}` : ""}`
          : text.trim()
            ? t(cat ? "gifts.picker.notInCatalog" : "gifts.picker.byName")
            : t("gifts.picker.any")}
      </span>
    </div>
  );
}

/** ¿El texto del campo es exactamente el nombre del regalo elegido? Entonces la lista muestra todo, no solo ese. */
function isChosenName(text: string, g: GiftView): boolean {
  return text.trim().toLowerCase() === primaryName(g).toLowerCase();
}
