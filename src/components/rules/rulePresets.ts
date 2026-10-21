import type { TFunction } from "i18next";
import type { ActionSpec, Conditions, Rule, Trigger } from "../../lib/types";
import { defaultConditions } from "./ruleDefaults";

export type PresetCategory = "new" | "thanks" | "interaction";

export interface RulePreset {
  key: string;
  category: PresetCategory;
  /** Lleva textos que el streamer debe completar (enlaces): se abre el editor en vez de guardarla tal cual. */
  needsEdit: boolean;
  build: (t: TFunction) => Rule;
}

/** Las reglas de plantilla tienen id fijo: así se sabe cuáles ya están añadidas y no se duplican. */
export const presetRuleId = (key: string) => `preset-${key}`;

const MIN = 60_000;
const HOUR = 60 * MIN;

const tts = (text: string): ActionSpec => ({ type: "tts", text });
/** Mensaje en pantalla (alerta del overlay): así lo ve el público aunque no se pueda escribir en el chat. */
const alert = (title: string): ActionSpec => ({ type: "overlayAlert", title, durationMs: 5000, wait: true });
/** Petición HTTP con un enlace de ejemplo que el streamer sustituye al personalizar la plantilla. */
const hook = (extra: Record<string, unknown>): ActionSpec => ({ type: "webhook", url: "https://ejemplo.com/hook", method: "GET", ...extra });

function preset(
  key: string,
  category: PresetCategory,
  trigger: Trigger,
  actions: (t: TFunction) => ActionSpec[],
  opts: { conditions?: Partial<Conditions>; costPoints?: number; priority?: number; needsEdit?: boolean; replacesGeneral?: boolean } = {},
): RulePreset {
  return {
    key,
    category,
    needsEdit: opts.needsEdit ?? false,
    build: (t) => ({
      id: presetRuleId(key),
      name: t(`presets.${key}.name`),
      enabled: true,
      trigger,
      conditions: { ...defaultConditions(), ...opts.conditions },
      plan: { mode: "sequence", steps: actions(t).map((action) => ({ delayMs: 0, action })) },
      ttlMs: 60_000,
      ...(opts.priority !== undefined ? { priority: opts.priority } : {}),
      ...(opts.costPoints !== undefined ? { costPoints: opts.costPoints } : {}),
      ...(opts.replacesGeneral ? { replacesGeneral: true } : {}),
    }),
  };
}

/** Reglas habituales de un streamer, listas para añadir y ajustar. */
export const RULE_PRESETS: RulePreset[] = [
  // Novedades: lo último que se ha añadido a HiveBuzz.
  preset("webhookGift", "new", { type: "gift", minCoins: 100 }, () => [
    hook({ method: "POST", bodyJson: { user: "{nickname}", gift: "{gift}", coins: "{coins}" }, retries: 1 }),
  ], { conditions: { globalCooldownMs: 1_000 }, needsEdit: true }),
  preset("webhookFollow", "new", { type: "follow" }, () => [hook({ url: "https://ejemplo.com/hook?user={user}", retries: 1 })], {
    conditions: { userCooldownMs: 6 * HOUR, globalCooldownMs: 1_000 },
    needsEdit: true,
  }),
  // Con grupos: sin elegir grupo la regla sería para todos, así que se abren en el editor (needsEdit).
  preset("vipGift", "new", { type: "gift", giftName: "Whale" }, (t) => [{ type: "overlayAlert", title: t("presets.vipGift.title"), text: "{gift}", durationMs: 8000, wait: true }], {
    conditions: { globalCooldownMs: 1_000 },
    priority: 80,
    replacesGeneral: true,
    needsEdit: true,
  }),
  preset("vipCommand", "new", { type: "command", command: "vip" }, (t) => [alert(t("presets.vipCommand.say"))], {
    conditions: { userCooldownMs: 30_000 },
    needsEdit: true,
  }),
  preset("fanClub", "new", { type: "command", command: "fan" }, (t) => [alert(t("presets.fanClub.say"))], {
    conditions: { userCooldownMs: 60_000, minTeamLevel: 1 },
  }),
  preset("gifterVoice", "new", { type: "command", command: "vozvip" }, (t) => [tts(t("presets.gifterVoice.say"))], {
    conditions: { userCooldownMs: 30_000, minGifterLevel: 5 },
  }),
  preset("jarFull", "new", { type: "goalReached", goalId: "" }, (t) => [tts(t("presets.jarFull.say"))], {
    priority: 60,
    needsEdit: true,
  }),
  // Agradecimientos en voz alta.
  preset("thanksGift", "thanks", { type: "gift" }, (t) => [tts(t("presets.thanksGift.say"))]),
  preset("bigGift", "thanks", { type: "gift", minCoins: 100 }, (t) => [alert(t("presets.bigGift.say"))], { priority: 50 }),
  preset("thanksFollow", "thanks", { type: "follow" }, (t) => [tts(t("presets.thanksFollow.say"))], {
    // Quien deja de seguir y vuelve a seguir no repite el agradecimiento.
    conditions: { userCooldownMs: 6 * HOUR, globalCooldownMs: 2_000 },
  }),
  preset("thanksShare", "thanks", { type: "share" }, (t) => [tts(t("presets.thanksShare.say"))], {
    conditions: { userCooldownMs: 10 * MIN },
  }),
  preset("thanksSub", "thanks", { type: "subscribe" }, (t) => [tts(t("presets.thanksSub.say"))], {
    priority: 40,
  }),
  // Interacción con el público.
  preset("likes", "interaction", { type: "like", every: 1000 }, (t) => [tts(t("presets.likes.say"))], {
    conditions: { globalCooldownMs: 60_000 },
  }),
  preset("sayForPoints", "interaction", { type: "command", command: "di" }, (t) => [tts(t("presets.sayForPoints.say"))], {
    conditions: { userCooldownMs: 30_000 },
    costPoints: 100,
  }),
  // Sonido por emote: sin sonido elegido (la regla pide uno antes de guardarse) y con el emote «cualquiera».
  preset("emoteSound", "interaction", { type: "emote" }, () => [{ type: "playSound", volume: 100, wait: true }], {
    conditions: { userCooldownMs: 10_000, globalCooldownMs: 1_000 },
    needsEdit: true,
  }),
  preset("modSay", "interaction", { type: "command", command: "anuncio" }, (t) => [tts(t("presets.modSay.say"))], {
    conditions: { rolesAny: ["moderator"], globalCooldownMs: 10_000 },
  }),
];

export const PRESET_CATEGORIES: PresetCategory[] = ["new", "thanks", "interaction"];
