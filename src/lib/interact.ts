import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import type { Step } from "./types";

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
  teamLevel: number;
  gifterLevel: number;
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

  getWheelConfig: () => invoke<WheelConfig>("get_wheel_config"),
  setWheelConfig: (config: WheelConfig) => invoke<WheelConfig>("set_wheel_config", { config }),
  spinWheelTest: () => invoke<void>("spin_wheel_test"),
  getPoll: () => invoke<PollView | null>("get_poll"),
  startPoll: (question: string, options: string[], durationSec: number) =>
    invoke<PollView>("start_poll", { question, options, durationSec }),
  stopPoll: () => invoke<PollView | null>("stop_poll"),
  clearPoll: () => invoke<void>("clear_poll"),

};
