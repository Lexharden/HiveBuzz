import { useTranslation } from "react-i18next";
import type { FieldDef, OverlayConfig } from "../../lib/types";
import { Checkbox, Field, NumberInput, Select, TextInput } from "../ui";

interface Props {
  fields: FieldDef[];
  config: OverlayConfig;
  onChange: (key: string, value: string | number | boolean) => void;
}

/** Formulario generado a partir del esquema que define el backend (única fuente de verdad). */
export function SchemaForm({ fields, config, onChange }: Props) {
  const { t } = useTranslation();
  const groups: { id: "style" | "behavior"; title: string }[] = [
    { id: "style", title: t("overlays.groupStyle") },
    { id: "behavior", title: t("overlays.groupBehavior") },
  ];

  return (
    <div className="space-y-5">
      {groups.map((g) => {
        const list = fields.filter((f) => f.group === g.id);
        if (list.length === 0) return null;
        return (
          <section key={g.id}>
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-500">{g.title}</h3>
            <div className="grid grid-cols-2 gap-3 md:grid-cols-3">
              {list.map((f) => (
                <FieldInput key={f.key} field={f} value={config[f.key]} onChange={(v) => onChange(f.key, v)} />
              ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}

function FieldInput({ field, value, onChange }: { field: FieldDef; value: string | number | boolean | undefined; onChange: (v: string | number | boolean) => void }) {
  const { t } = useTranslation();
  const label = t(field.label);

  switch (field.kind) {
    case "color": {
      const v = typeof value === "string" ? value : field.default;
      return (
        <Field label={label}>
          <div className="flex items-center gap-2">
            <input
              type="color"
              value={v}
              onChange={(e) => onChange(e.target.value)}
              className="h-8 w-10 cursor-pointer rounded border border-zinc-700 bg-zinc-950 p-0.5"
            />
            <span className="font-mono text-xs text-zinc-400">{v}</span>
          </div>
        </Field>
      );
    }
    case "number":
      return (
        <Field label={label}>
          <NumberInput
            value={typeof value === "number" ? value : field.default}
            min={field.min}
            max={field.max}
            step={field.step}
            // Un campo vacío no cambia nada: el backend siempre exige un número válido.
            onChange={(n) => n !== null && onChange(n)}
          />
        </Field>
      );
    case "select":
      return (
        <Field label={label}>
          <Select
            value={typeof value === "string" ? value : field.default}
            onChange={onChange}
            options={field.options.map(([val, key]) => ({ value: val, label: t(key) }))}
          />
        </Field>
      );
    case "bool":
      return (
        <div className="flex items-end pb-1.5">
          <Checkbox checked={typeof value === "boolean" ? value : field.default} onChange={onChange} label={label} />
        </div>
      );
    case "text":
      return (
        <Field label={label}>
          <TextInput value={typeof value === "string" ? value : field.default} maxLength={field.maxLen} onChange={onChange} />
        </Field>
      );
  }
}
