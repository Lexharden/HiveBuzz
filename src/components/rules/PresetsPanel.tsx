import { useTranslation } from "react-i18next";
import type { Rule } from "../../lib/types";
import { Btn, Card } from "../ui";
import { PRESET_CATEGORIES, presetRuleId, RULE_PRESETS, type RulePreset } from "./rulePresets";

interface Props {
  rules: Rule[];
  /** Guarda la regla tal cual. */
  onAdd: (rules: Rule[]) => void;
  /** Abre el editor con la regla rellenada (para las que llevan enlaces que completar). */
  onCustomize: (rule: Rule) => void;
  onClose: () => void;
}

/** Plantillas de reglas habituales: se añaden con un clic y luego se editan como cualquier otra. */
export function PresetsPanel({ rules, onAdd, onCustomize, onClose }: Props) {
  const { t } = useTranslation();
  const added = new Set(rules.map((r) => r.id));
  const isAdded = (p: RulePreset) => added.has(presetRuleId(p.key));

  return (
    <Card title={t("presets.title")} hint={t("presets.hint")} actions={<Btn onClick={onClose}>{t("presets.close")}</Btn>}>
      <div className="space-y-4">
        {PRESET_CATEGORIES.map((cat) => {
          const items = RULE_PRESETS.filter((p) => p.category === cat);
          const pending = items.filter((p) => !isAdded(p) && !p.needsEdit);
          return (
            <section key={cat}>
              <div className="mb-2 flex items-center justify-between">
                <h3 className="text-sm font-semibold text-zinc-200">{t(`presets.category.${cat}`)}</h3>
                {pending.length > 1 && (
                  <Btn onClick={() => onAdd(pending.map((p) => p.build(t)))}>{t("presets.addAll", { n: pending.length })}</Btn>
                )}
              </div>
              <ul className="grid grid-cols-1 gap-2 md:grid-cols-2">
                {items.map((p) => (
                  <li key={p.key} className="flex items-start justify-between gap-3 rounded-lg border border-zinc-800 bg-zinc-950/50 px-3 py-2">
                    <div className="min-w-0">
                      <div className="text-sm font-semibold">{t(`presets.${p.key}.name`)}</div>
                      <div className="text-xs text-zinc-400">{t(`presets.${p.key}.desc`)}</div>
                      <div className="mt-1 flex flex-wrap gap-1">
                        {p.needsBot && <span className="rounded bg-sky-500/15 px-1.5 text-[10px] text-sky-300">{t("presets.needsBot")}</span>}
                        {p.needsEdit && <span className="rounded bg-amber-500/15 px-1.5 text-[10px] text-amber-300">{t("presets.needsEdit")}</span>}
                      </div>
                    </div>
                    {isAdded(p) ? (
                      <span className="shrink-0 text-xs text-emerald-400">✓ {t("presets.added")}</span>
                    ) : p.needsEdit ? (
                      <Btn onClick={() => onCustomize(p.build(t))}>{t("presets.customize")}</Btn>
                    ) : (
                      <Btn variant="primary" onClick={() => onAdd([p.build(t)])}>
                        + {t("presets.add")}
                      </Btn>
                    )}
                  </li>
                ))}
              </ul>
            </section>
          );
        })}
      </div>
    </Card>
  );
}
