import type { FontFamily, Media, Rule, TimerView } from "../../lib/types";
import { presetRuleId } from "./rulePresets";

/** Una plantilla que conviene probar y el motivo (clave de i18n + valores) que se le explica al streamer. */
export interface Suggestion {
  /** Clave de la plantilla (`RULE_PRESETS`). */
  key: string;
  reason: { key: string; vars?: Record<string, string | number> };
}

export interface SuggestionInput {
  rules: Rule[];
  timers: TimerView[];
  media: Media[];
  fonts: FontFamily[];
}

const usesAction = (rules: Rule[], type: string) => rules.some((r) => r.enabled && r.plan.steps.some((s) => s.action.type === type));
/** Reglas de regalo que muestran una alerta (con o sin capas). */
const giftAlertRules = (rules: Rule[]) => rules.filter((r) => r.trigger.type === "gift" && r.plan.steps.some((s) => s.action.type === "overlayAlert"));
const hasLayers = (rules: Rule[]) =>
  rules.some((r) => r.plan.steps.some((s) => s.action.type === "overlayAlert" && Array.isArray(s.action.layers) && s.action.layers.length > 0));

/**
 * Mira lo que el streamer ya tiene (timers, imágenes, fuentes, reglas) y propone las plantillas de las
 * novedades que le sacarían partido y aún no usa. Nunca propone una plantilla ya añadida.
 */
export function suggestPresets({ rules, timers, media, fonts }: SuggestionInput): Suggestion[] {
  const added = new Set(rules.map((r) => r.id));
  const out: Suggestion[] = [];
  const add = (s: Suggestion) => {
    if (!added.has(presetRuleId(s.key)) && !out.some((o) => o.key === s.key)) out.push(s);
  };

  const timer = timers[0];
  if (timer) {
    const endsWithRule = rules.some((r) => r.trigger.type === "timerEnded");
    if (!endsWithRule) add({ key: "timerEnded", reason: { key: "presets.suggest.timerEnded", vars: { name: timer.config.name } } });
    if (!usesAction(rules, "timerControl") && timer.config.mode !== "stopwatch") {
      add({ key: "timerFollowAdd", reason: { key: "presets.suggest.timerFollowAdd", vars: { name: timer.config.name } } });
    }
  }

  // Con imágenes o fuentes propias ya se pueden hacer alertas con capas; si ninguna regla las usa, se sugiere.
  if (!hasLayers(rules)) {
    const images = media.filter((m) => m.kind === "image").length;
    if (fonts.length > 0) add({ key: "alertLayers", reason: { key: "presets.suggest.layersFonts", vars: { n: fonts.length } } });
    else if (images > 0) add({ key: "alertLayers", reason: { key: "presets.suggest.layersImages", vars: { n: images } } });
    else if (giftAlertRules(rules).length > 0) {
      add({ key: "alertLayers", reason: { key: "presets.suggest.layersPlain" } });
    }
  }
  return out;
}
