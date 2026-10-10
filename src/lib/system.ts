import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";

export interface AppPrefs {
  closeToTray: boolean;
  startMinimized: boolean;
  language: "es" | "en";
  autoUpdateCheck: boolean;
}

export interface LogEntry {
  tsMs: number;
  level: "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE";
  target: string;
  message: string;
}

export interface StreamSummary {
  id: number;
  endedMs: number;
  coins: number;
  peakViewers: number;
  giftsTotal: number;
  chats: number;
  likes: number;
  follows: number;
  shares: number;
  subscribers: number;
}

export interface StreamStats extends StreamSummary {
  gifts: { name: string; count: number; coins: number }[];
  donors: { userId: string; uniqueId: string; nickname: string; coins: number; gifts: number }[];
}

export interface ProfileInfo {
  id: string;
  name: string;
  updatedMs: number;
  ruleCount: number;
  overlayCount: number;
}

export interface BackupSummary {
  rules: number;
  goals: number;
  timers: number;
  overlays: number;
  sounds: number;
  media: number;
  profiles: number;
  skipped: string[];
}

export interface UpdateInfo {
  available: boolean;
  version: string | null;
  current: string;
  notes: string | null;
}

export interface ReleaseInfo {
  tag: string;
  version: string;
  name: string;
  notes: string;
  publishedAt: string | null;
  prerelease: boolean;
  /** Trae `latest.json` firmado: se instala desde la app. */
  installable: boolean;
  url: string;
  relation: "newer" | "current" | "older" | "unknown";
}

export interface UpdateProgress {
  downloaded: number;
  total: number | null;
}

export interface SongConfig {
  enabled: boolean;
  commands: string[];
  nowPlayingCommands: string[];
  minRole: "everyone" | "follower" | "subscriber" | "moderator";
  costPoints: number;
  perUserLimit: number;
  userCooldownS: number;
  maxDurationS: number;
  blockedTerms: string[];
  blockedUsers: string[];
  reply: boolean;
}

export interface SpotifyConfig {
  clientId: string;
  song: SongConfig;
}

export interface SpotifyStatus {
  connected: boolean;
  clientIdSet: boolean;
  usesBuiltinApp: boolean;
  redirectUri: string;
  pendingRequests: number;
}

export interface SpotifyTrack {
  id: string;
  uri: string;
  name: string;
  artists: string[];
  durationMs: number;
  image: string | null;
}

export const systemApi = {
  getPrefs: () => invoke<AppPrefs>("get_app_prefs"),
  setPrefs: (prefs: AppPrefs) => invoke<AppPrefs>("set_app_prefs", { prefs }),
  getAutostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (enabled: boolean) => invoke<boolean>("set_autostart", { enabled }),

  getLogs: (minLevel: string, limit: number) => invoke<LogEntry[]>("get_logs", { minLevel, limit }),
  clearLogs: () => invoke<void>("clear_logs"),
  exportLogs: async (): Promise<boolean> => {
    const path = await save({ defaultPath: "hivebuzz-logs.txt", filters: [{ name: "Texto", extensions: ["txt"] }] });
    if (!path) return false;
    await invoke<void>("export_logs", { path });
    return true;
  },

  listStreams: () => invoke<StreamSummary[]>("list_streams", { limit: 100 }),
  getStream: (id: number) => invoke<StreamStats>("get_stream", { id }),
  deleteStream: (id: number) => invoke<boolean>("delete_stream", { id }),

  listProfiles: () => invoke<ProfileInfo[]>("list_profiles"),
  activeProfile: () => invoke<string | null>("active_profile"),
  saveProfile: (id: string | null, name: string) => invoke<ProfileInfo>("save_profile", { id, name }),
  applyProfile: (id: string) => invoke<void>("apply_profile", { id }),
  renameProfile: (id: string, name: string) => invoke<void>("rename_profile", { id, name }),
  deleteProfile: (id: string) => invoke<boolean>("delete_profile", { id }),

  exportConfig: async (): Promise<BackupSummary | null> => {
    const path = await save({ defaultPath: "hivebuzz-config.zip", filters: [{ name: "HiveBuzz", extensions: ["zip"] }] });
    if (!path) return null;
    return invoke<BackupSummary>("export_config", { path });
  },
  importConfig: async (): Promise<BackupSummary | null> => {
    const path = await open({ multiple: false, filters: [{ name: "HiveBuzz", extensions: ["zip"] }] });
    if (typeof path !== "string") return null;
    return invoke<BackupSummary>("import_config", { path });
  },
  hasPendingImport: () => invoke<boolean>("has_pending_import"),
  cancelPendingImport: () => invoke<void>("cancel_pending_import"),
  takeImportNotice: () => invoke<string | null>("take_import_notice"),
  restartApp: () => invoke<void>("restart_app"),

  checkUpdate: () => invoke<UpdateInfo>("check_update"),
  installUpdate: () => invoke<void>("install_update"),
  listReleases: () => invoke<ReleaseInfo[]>("list_releases"),
  prepareRelease: (tag: string) => invoke<UpdateInfo>("prepare_release", { tag }),

  spotifyGetConfig: () => invoke<SpotifyConfig>("spotify_get_config"),
  spotifySetConfig: (config: SpotifyConfig) => invoke<SpotifyConfig>("spotify_set_config", { config }),
  spotifyStatus: () => invoke<SpotifyStatus>("spotify_status"),
  spotifyConnect: () => invoke<void>("spotify_connect"),
  spotifyDisconnect: () => invoke<void>("spotify_disconnect"),
  spotifyQueueTest: (query: string) => invoke<SpotifyTrack>("spotify_queue_test", { query }),
};

export const onUpdateProgress = (cb: (p: UpdateProgress) => void): Promise<UnlistenFn> =>
  listen<UpdateProgress>("update-progress", (e) => cb(e.payload));
