import type { TFunction } from "i18next";
import { describe, expect, it } from "vitest";
import en from "../../src/i18n/en.json";
import es from "../../src/i18n/es.json";
import { suggestPresets } from "../../src/components/rules/presetSuggestions";
import { PRESET_CATEGORIES, presetRuleId, RULE_PRESETS } from "../../src/components/rules/rulePresets";
import type { FontFamily, Media, Rule, TimerView } from "../../src/lib/types";

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

describe("plantillas de las novedades", () => {
  const rule = (key: string) => RULE_PRESETS.find((p) => p.key === key)?.build(translator(es as Tree));

  it("la alerta con capas lleva texto, imagen del regalo y cantidad, todo traducido y sin enlaces que completar", () => {
    const p = RULE_PRESETS.find((x) => x.key === "alertLayers");
    expect(p?.needsEdit).toBe(false);
    const action = rule("alertLayers")?.plan.steps[0]?.action as unknown as { layers: { type: string; text?: string; imageUrl?: string; x: number; y: number }[] };
    expect(action.layers.map((l) => l.type)).toEqual(["text", "image", "text"]);
    expect(action.layers[0]?.text).not.toMatch(/^presets\./);
    expect(action.layers[1]?.imageUrl).toBe("{giftimage}");
    for (const l of action.layers) expect(l.x >= 0 && l.x <= 100 && l.y >= 0 && l.y <= 100).toBe(true);
    expect(action.layers.length).toBeLessThanOrEqual(8);
  });

  it("las de timers piden elegir el timer antes de guardarse", () => {
    for (const key of ["timerEnded", "timerFollowAdd"]) expect(RULE_PRESETS.find((p) => p.key === key)?.needsEdit, key).toBe(true);
    expect(rule("timerEnded")?.trigger).toEqual({ type: "timerEnded", timerId: "" });
    expect(rule("timerFollowAdd")?.plan.steps[0]?.action).toMatchObject({ type: "timerControl", op: "add", seconds: 30, timerId: "" });
  });
});

describe("sugerencias inteligentes", () => {
  const timer = (mode: "countdown" | "stopwatch" = "countdown"): TimerView => ({ config: { id: "t", name: "Subathon", mode, startSeconds: 3600, extensions: [] }, status: "idle", remainingMs: 0 });
  const image = { id: "m", name: "logo", file: "m.png", kind: "image", createdMs: 0 } as Media;
  const video = { ...image, kind: "video" } as Media;
  const font: FontFamily = { family: "Anton", weights: [400] };
  const base = { rules: [] as Rule[], timers: [] as TimerView[], media: [] as Media[], fonts: [] as FontFamily[] };
  const ruleWith = (over: Partial<Rule> & { action?: Record<string, unknown> }): Rule => {
    const r = RULE_PRESETS[0]!.build(translator(es as Tree));
    return { ...r, id: "mia", trigger: { type: "follow" }, ...over, plan: { mode: "sequence", steps: [{ delayMs: 0, action: { type: "overlayAlert", title: "x", ...over.action } }] } } as Rule;
  };
  const keys = (i: typeof base) => suggestPresets(i).map((s) => s.key);

  it("sin timers, imágenes ni fuentes no sugiere nada", () => {
    expect(suggestPresets(base)).toEqual([]);
  });

  it("con un timer sin aviso de fin ni reglas que lo muevan, sugiere ambas con su nombre", () => {
    const got = suggestPresets({ ...base, timers: [timer()] });
    expect(got.map((s) => s.key)).toEqual(["timerEnded", "timerFollowAdd"]);
    expect(got[0]?.reason.vars).toEqual({ name: "Subathon" });
  });

  it("un cronómetro no sugiere sumarle tiempo (no admite extensiones)", () => {
    expect(keys({ ...base, timers: [timer("stopwatch")] })).toEqual(["timerEnded"]);
  });

  it("no repite lo que ya está cubierto: regla de fin, regla timerControl o plantilla ya añadida", () => {
    const ended = ruleWith({ trigger: { type: "timerEnded", timerId: "t" } });
    expect(keys({ ...base, timers: [timer()], rules: [ended] })).toEqual(["timerFollowAdd"]);
    const control = ruleWith({ enabled: true, action: { type: "timerControl", op: "add" } });
    expect(keys({ ...base, timers: [timer()], rules: [ended, control] })).toEqual([]);
    const added = ruleWith({ id: presetRuleId("timerEnded"), trigger: { type: "follow" } });
    expect(keys({ ...base, timers: [timer()], rules: [added] })).toEqual(["timerFollowAdd"]);
  });

  it("con imágenes o fuentes y sin alertas con capas, sugiere la alerta con capas (las fuentes mandan en el motivo)", () => {
    expect(suggestPresets({ ...base, media: [image, video] })[0]).toMatchObject({ key: "alertLayers", reason: { key: "presets.suggest.layersImages", vars: { n: 1 } } });
    expect(suggestPresets({ ...base, media: [image], fonts: [font] })[0]?.reason.key).toBe("presets.suggest.layersFonts");
    expect(keys({ ...base, media: [video] })).toEqual([]);
  });

  it("si una regla ya usa capas, o la plantilla ya está añadida, no la sugiere", () => {
    const withLayers = ruleWith({ action: { layers: [{ type: "text", text: "hola", x: 50, y: 50 }] } });
    expect(keys({ ...base, media: [image], rules: [withLayers] })).toEqual([]);
    const added = ruleWith({ id: presetRuleId("alertLayers") });
    expect(keys({ ...base, media: [image], rules: [added] })).toEqual([]);
  });

  it("con alertas de regalo sin capas sugiere probarlas aunque no haya imágenes ni fuentes", () => {
    const plain = ruleWith({ trigger: { type: "gift" } });
    expect(suggestPresets({ ...base, rules: [plain] })[0]?.reason.key).toBe("presets.suggest.layersPlain");
  });

  for (const [lang, tree] of [["es", es], ["en", en]] as const) {
    it(`en ${lang} todos los motivos están traducidos`, () => {
      const t = translator(tree as Tree);
      const all = suggestPresets({ ...base, timers: [timer()], media: [image] }).concat(suggestPresets({ ...base, fonts: [font] }), suggestPresets({ ...base, rules: [ruleWith({ trigger: { type: "gift" } })] }));
      expect(all.length).toBeGreaterThan(3);
      for (const s of all) expect(t(s.reason.key), s.reason.key).not.toBe(s.reason.key);
      expect(t("presets.suggest.title")).not.toBe("presets.suggest.title");
    });
  }
});
