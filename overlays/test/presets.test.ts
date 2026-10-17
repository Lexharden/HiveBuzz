import type { TFunction } from "i18next";
import { describe, expect, it } from "vitest";
import en from "../../src/i18n/en.json";
import es from "../../src/i18n/es.json";
import { PRESET_CATEGORIES, presetRuleId, RULE_PRESETS } from "../../src/components/rules/rulePresets";

type Tree = { [k: string]: string | Tree };

/** Traductor mínimo: busca la clave en el árbol y devuelve la propia clave si falta (así se detecta). */
const translator = (tree: Tree): TFunction =>
  ((key: string, opts?: { defaultValue?: string }) => {
    const found = key.split(".").reduce<string | Tree | undefined>((node, part) => (typeof node === "object" ? node[part] : undefined), tree);
    return typeof found === "string" ? found : (opts?.defaultValue ?? key);
  }) as unknown as TFunction;

describe("plantillas de reglas", () => {
  it("las claves y los ids son únicos y todas tienen categoría conocida", () => {
    const keys = RULE_PRESETS.map((p) => p.key);
    expect(new Set(keys).size).toBe(keys.length);
    expect(new Set(keys.map(presetRuleId)).size).toBe(keys.length);
    for (const p of RULE_PRESETS) expect(PRESET_CATEGORIES, p.key).toContain(p.category);
  });

  for (const [lang, tree] of [["es", es], ["en", en]] as const) {
    it(`en ${lang} todas tienen nombre, descripción y textos traducidos`, () => {
      const t = translator(tree as Tree);
      for (const p of RULE_PRESETS) {
        const rule = p.build(t);
        expect(rule.name, p.key).not.toBe(`presets.${p.key}.name`);
        expect(t(`presets.${p.key}.desc`), p.key).not.toBe(`presets.${p.key}.desc`);
        for (const step of rule.plan.steps) {
          for (const v of Object.values(step.action)) {
            if (typeof v === "string") expect(v, `${p.key}: texto sin traducir`).not.toMatch(/^presets\./);
          }
        }
        if (p.needsEdit && p.category === "new") expect(t(`presets.${p.key}.edit`), p.key).not.toBe(`presets.${p.key}.edit`);
      }
      expect(t("presets.category.new")).not.toBe("presets.category.new");
    });
  }

  it("las novedades usan lo nuevo: webhooks con reintento, niveles de fan y de donador, meta cumplida", () => {
    const rule = (key: string) => RULE_PRESETS.find((p) => p.key === key)?.build(translator(es as Tree));
    expect(rule("webhookGift")?.plan.steps[0]?.action).toMatchObject({ type: "webhook", method: "POST", retries: 1 });
    expect(rule("webhookFollow")?.plan.steps[0]?.action).toMatchObject({ type: "webhook", method: "GET" });
    expect(rule("fanClub")?.conditions.minTeamLevel).toBe(1);
    expect(rule("gifterVoice")?.conditions.minGifterLevel).toBe(5);
    expect(rule("jarFull")?.trigger.type).toBe("goalReached");
  });

  it("las que llevan enlace o meta piden personalizarse antes de guardarse", () => {
    for (const key of ["webhookGift", "webhookFollow", "jarFull"]) {
      expect(RULE_PRESETS.find((p) => p.key === key)?.needsEdit, key).toBe(true);
    }
  });
});
