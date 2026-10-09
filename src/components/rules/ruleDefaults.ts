import type { ActionSpec, Conditions, Rule, Trigger, TriggerType } from "../../lib/types";

export const TRIGGER_TYPES: TriggerType[] = [
  "gift",
  "follow",
  "share",
  "subscribe",
  "join",
  "subEmote",
  "like",
  "command",
  "keyword",
  "goalReached",
  "timerEnded",
  "api",
];

export function defaultTrigger(type: TriggerType): Trigger {
  switch (type) {
    case "gift":
      return { type };
    case "like":
      return { type, every: 100 };
    case "command":
      return { type, command: "!comando" };
    case "keyword":
      return { type, keywords: [], wholeWord: true };
    case "goalReached":
      return { type, goalId: "" };
    case "timerEnded":
      return { type, timerId: "" };
    case "api":
      return { type, name: "mi-accion" };
    default:
      return { type };
  }
}

export function defaultConditions(): Conditions {
  return { globalCooldownMs: 0, userCooldownMs: 0, rolesAny: [], probability: 100 };
}

export function defaultAction(type: string): ActionSpec {
  switch (type) {
    case "playSound":
      return { type, volume: 100, wait: true };
    case "overlayAlert":
      return { type, title: "{nickname}", text: "", durationMs: 5000, wait: true };
    case "tts":
      return { type, text: "Gracias {nickname}" };
    case "goalAdjust":
      return { type, goalId: "", op: "add", amount: 1 };
    case "timerControl":
      return { type, timerId: "", op: "add", seconds: 60 };
    case "botMessage":
      return { type, text: "¡Gracias {nickname}!" };
    case "pointsAdjust":
      return { type, amount: 10, target: "{user}" };
    case "startPoll":
      return { type, question: "", options: ["", ""], durationS: 60 };
    case "webhook":
      return { type, url: "https://", method: "POST", bodyJson: { user: "{nickname}" } };
    case "tcpSend":
      return { type, host: "127.0.0.1", port: 25000, message: "{nickname}", newline: true };
    case "wsSend":
      return { type, url: "ws://127.0.0.1:8080", message: "{nickname}" };
    case "pressKeys":
      return { type, keys: "f5", holdMs: 50, targetWindows: [] };
    case "obs":
      return { type, action: "setScene", scene: "" };
    default:
      return { type };
  }
}

export function newRule(): Rule {
  return {
    id: crypto.randomUUID(),
    name: "",
    enabled: true,
    trigger: defaultTrigger("gift"),
    conditions: defaultConditions(),
    plan: { mode: "sequence", steps: [{ delayMs: 0, action: defaultAction("overlayAlert") }] },
    ttlMs: 60_000,
  };
}

/** Texto corto que describe el trigger, para la lista de reglas. */
export function triggerSummary(t: Trigger): string {
  switch (t.type) {
    case "gift": {
      const parts = [t.giftName, t.giftId !== undefined ? `#${t.giftId}` : null, t.minCoins !== undefined ? `≥${t.minCoins}🪙` : null];
      return parts.filter(Boolean).join(" · ");
    }
    case "like":
      return `cada ${t.every}`;
    case "command":
      return t.command;
    case "keyword":
      return t.keywords.join(", ");
    case "goalReached":
      return t.goalId;
    case "timerEnded":
      return t.timerId;
    case "api":
      return t.name;
    default:
      return "";
  }
}
