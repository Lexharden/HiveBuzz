import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import type { Conditions, Step } from "./types";

export interface PointsConfig {
  enabled: boolean;
  currencyName: string;
  watchPoints: number;
  watchIntervalMinutes: number;
  commentPoints: number;
  commentCooldownMs: number;
  likePoints: number;
  likeEvery: number;
  sharePoints: number;
  followPoints: number;
  subscribePoints: number;
  pointsPerCoin: number;
  subscriberMultiplier: number;
  pointsCommand: string;
  topCommand: string;
  topSize: number;
  countSimulated: boolean;
}

export interface Viewer {
  userId: string;
  uniqueId: string;
  nickname: string;
  points: number;
  totalEarned: number;
  totalSpent: number;
  coinsGifted: number;
  comments: number;
  lastSeenMs: number;
}

export interface ViewerPage {
  items: Viewer[];
  total: number;
}

export interface ImportReport {
  imported: number;
  skipped: number;
  [k: string]: unknown;
}

export interface BotCommand {
  id: string;
  enabled: boolean;
  names: string[];
  responses: string[];
  conditions: Conditions;
}

export interface TimedMessage {
  id: string;
  enabled: boolean;
  text: string;
  everyMinutes: number;
  minChatMessages: number;
}

export interface Thanks {
  enabled: boolean;
  template: string;
  minCoins: number;
  userCooldownMs: number;
}

export interface BotConfig {
  enabled: boolean;
  minIntervalMs: number;
  commands: BotCommand[];
  keywordReplies: unknown[];
  timedMessages: TimedMessage[];
  thanks: { gift: Thanks; follow: Thanks; share: Thanks; subscribe: Thanks };
  builtin: { points: string; top: string; redeemed: string; denied: string };
}

export interface BotLogEntry {
  ts: number;
  text: string;
  source: string;
  status: { state: "sent" | "failed" | "dropped"; reason?: string };
}

export interface WheelSegment {
  id: string;
  label: string;
  weight: number;
  color: string;
  plan: { mode: string; steps: Step[] };
}

export interface WheelConfig {
  segments: WheelSegment[];
  spinMs: number;
  announce: string;
}

export interface PollView {
  id: number;
  question: string;
  options: { label: string; votes: number }[];
  total: number;
  endsAtMs: number;
  ended: boolean;
  winners: number[];
}

export const interactApi = {
  getPointsConfig: () => invoke<PointsConfig>("get_points_config"),
  setPointsConfig: (config: PointsConfig) => invoke<PointsConfig>("set_points_config", { config }),
  listViewers: (search: string, sort: string, limit: number, offset: number) =>
    invoke<ViewerPage>("list_viewers", { search, sort, limit, offset }),
  adjustViewerPoints: (userId: string, delta: number) => invoke<number>("adjust_viewer_points", { userId, delta }),
  deleteViewer: (userId: string) => invoke<boolean>("delete_viewer", { userId }),
  exportViewersCsv: async (): Promise<boolean> => {
    const path = await save({ defaultPath: "espectadores.csv", filters: [{ name: "CSV", extensions: ["csv"] }] });
    if (!path) return false;
    await invoke<void>("export_viewers_csv", { path });
    return true;
  },
  importViewersCsv: (path: string, mode: "replace" | "add") => invoke<ImportReport>("import_viewers_csv", { path, mode }),

  getBotConfig: () => invoke<BotConfig>("get_bot_config"),
  setBotConfig: (config: BotConfig) => invoke<BotConfig>("set_bot_config", { config }),
  getBotLog: () => invoke<BotLogEntry[]>("get_bot_log"),
  clearBotLog: () => invoke<void>("clear_bot_log"),
  botSay: (text: string) => invoke<void>("bot_say", { text }),

  getWheelConfig: () => invoke<WheelConfig>("get_wheel_config"),
  setWheelConfig: (config: WheelConfig) => invoke<WheelConfig>("set_wheel_config", { config }),
  spinWheelTest: () => invoke<void>("spin_wheel_test"),
  getPoll: () => invoke<PollView | null>("get_poll"),
  startPoll: (question: string, options: string[], durationSec: number) =>
    invoke<PollView>("start_poll", { question, options, durationSec }),
  stopPoll: () => invoke<PollView | null>("stop_poll"),
  clearPoll: () => invoke<void>("clear_poll"),

  hasTiktokSession: () => invoke<boolean>("has_tiktok_session"),
  tiktokLoginStart: () => invoke<void>("tiktok_login_start"),
  tiktokLoginFinish: () => invoke<boolean>("tiktok_login_finish"),
  tiktokLogout: () => invoke<void>("tiktok_logout"),
};
