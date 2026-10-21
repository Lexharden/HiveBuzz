import { useTranslation } from "react-i18next";
import type { ViewerGroup } from "../../lib/types";
import { Field } from "../ui";

/** Elige varios grupos (casillas). Los ids que ya no existen se muestran en rojo para poder quitarlos. */
export function GroupPicker({
  label,
  hint,
  groups,
  value,
  onChange,
}: {
  label: string;
  hint?: string;
  groups: ViewerGroup[];
  value: string[];
  onChange: (ids: string[]) => void;
}) {
  const { t } = useTranslation();
  const known = new Set(groups.map((g) => g.id));
  const missing = value.filter((id) => !known.has(id));
  const toggle = (id: string, on: boolean) => onChange(on ? [...value, id] : value.filter((x) => x !== id));

  return (
    <Field label={label} hint={hint}>
      <div className="flex flex-wrap gap-2">
        {groups.length === 0 && missing.length === 0 && <span className="text-xs text-zinc-500">{t("groups.noneYet")}</span>}
        {groups.map((g) => {
          const on = value.includes(g.id);
          return (
            <button
              key={g.id}
              type="button"
              onClick={() => toggle(g.id, !on)}
              aria-pressed={on}
              className={`rounded-full border px-3 py-1 text-xs ${on ? "border-amber-400 bg-amber-400/15 text-amber-200" : "border-zinc-700 text-zinc-400 hover:border-zinc-500"}`}
            >
              {g.name} <span className="text-zinc-500">({g.members.length})</span>
            </button>
          );
        })}
        {missing.map((id) => (
          <button
            key={id}
            type="button"
            onClick={() => toggle(id, false)}
            title={t("groups.deletedHint")}
            className="rounded-full border border-rose-500/50 bg-rose-500/10 px-3 py-1 text-xs text-rose-300"
          >
            ✕ {t("groups.deleted")}
          </button>
        ))}
      </div>
    </Field>
  );
}
