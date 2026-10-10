import type { TFunction } from "i18next";
import type { ActionSpec, Conditions, Rule, Trigger } from "../../lib/types";
import { defaultConditions } from "./ruleDefaults";

export type PresetCategory = "thanks" | "chat" | "interaction";

export interface RulePreset {
  key: string;
  category: PresetCategory;
  /** Usa el bot de chat (hay que iniciar sesión en «Bot y puntos»). */
  needsBot: boolean;
  /** Lleva textos que el streamer debe completar (enlaces): se abre el editor en vez de guardarla tal cual. */
  needsEdit: boolean;
  build: (t: TFunction) => Rule;
}

/** Las reglas de plantilla tienen id fijo: así se sabe cuáles ya están añadidas y no se duplican. */
export const presetRuleId = (key: string) => `preset-${key}`;

const MIN = 60_000;
const HOUR = 60 * MIN;

const tts = (text: string): ActionSpec => ({ type: "tts", text });
const bot = (text: string): ActionSpec => ({ type: "botMessage", text });

function preset(
  key: string,
  category: PresetCategory,
  trigger: Trigger,
  actions: (t: TFunction) => ActionSpec[],
  opts: { conditions?: Partial<Conditions>; costPoints?: number; priority?: number; needsEdit?: boolean } = {},
): RulePreset {
  const needsBot = actions(((k: string) => k) as unknown as TFunction).some((a) => a.type === "botMessage");
  return {
    key,
    category,
    needsBot,
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
    }),
  };
}

/** Reglas habituales de un streamer, listas para añadir y ajustar. */
export const RULE_PRESETS: RulePreset[] = [
  // Agradecimientos en voz alta.
  preset("thanksGift", "thanks", { type: "gift" }, (t) => [tts(t("presets.thanksGift.say"))]),
  preset("bigGift", "thanks", { type: "gift", minCoins: 100 }, (t) => [bot(t("presets.bigGift.say"))], { priority: 50 }),
  preset("thanksFollow", "thanks", { type: "follow" }, (t) => [tts(t("presets.thanksFollow.say"))], {
    // Quien deja de seguir y vuelve a seguir no repite el agradecimiento.
    conditions: { userCooldownMs: 6 * HOUR, globalCooldownMs: 2_000 },
  }),
  preset("thanksShare", "thanks", { type: "share" }, (t) => [tts(t("presets.thanksShare.say"))], {
    conditions: { userCooldownMs: 10 * MIN },
  }),
  preset("thanksSub", "thanks", { type: "subscribe" }, (t) => [tts(t("presets.thanksSub.say")), bot(t("presets.thanksSub.chat"))], {
    priority: 40,
  }),
  // Bot de chat.
  preset("welcome", "chat", { type: "join" }, (t) => [bot(t("presets.welcome.say"))], {
    conditions: { userCooldownMs: 12 * HOUR, globalCooldownMs: 20_000, rolesAny: ["follower"] },
  }),
  preset("hello", "chat", { type: "keyword", keywords: ["hola", "buenas", "hello", "hi"], wholeWord: true }, (t) => [bot(t("presets.hello.say"))], {
    conditions: { userCooldownMs: 30 * MIN, globalCooldownMs: 5_000 },
  }),
  preset("cmdDiscord", "chat", { type: "command", command: "discord" }, (t) => [bot(t("presets.cmdDiscord.say"))], {
    conditions: { globalCooldownMs: 30_000 },
    needsEdit: true,
  }),
  preset("cmdSocials", "chat", { type: "command", command: "redes" }, (t) => [bot(t("presets.cmdSocials.say"))], {
    conditions: { globalCooldownMs: 30_000 },
    needsEdit: true,
  }),
  preset("cmdCommands", "chat", { type: "command", command: "comandos" }, (t) => [bot(t("presets.cmdCommands.say"))], {
    conditions: { globalCooldownMs: 30_000 },
  }),
  // Interacción con el público.
  preset("likes", "interaction", { type: "like", every: 1000 }, (t) => [tts(t("presets.likes.say"))], {
    conditions: { globalCooldownMs: 60_000 },
  }),
  preset("sayForPoints", "interaction", { type: "command", command: "di" }, (t) => [tts(t("presets.sayForPoints.say"))], {
    conditions: { userCooldownMs: 30_000 },
    costPoints: 100,
  }),
  preset("modSay", "interaction", { type: "command", command: "anuncio" }, (t) => [tts(t("presets.modSay.say"))], {
    conditions: { rolesAny: ["moderator"], globalCooldownMs: 10_000 },
  }),
];

export const PRESET_CATEGORIES: PresetCategory[] = ["thanks", "chat", "interaction"];
