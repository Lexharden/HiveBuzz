import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  AppInfo,
  Counters,
  DonorEntry,
  FiredReport,
  Goal,
  InstallProgress,
  LeaderboardScope,
  LiveEvent,
  Media,
  OverlayConfig,
  OverlayDef,
  Platform,
  PlatformStatus,
  QueueInfo,
  Rule,
  SimKind,
  Sound,
  TimerConfig,
  TimerView,
  TtsConfig,
  TtsStatus,
  MicStatus,
  VoiceInfo,
} from "./types";

export const api = {
  // Conexión y ajustes
  connect: (platform: Platform, username: string) => invoke<void>("connect", { platform, username }),
  disconnect: (platform: Platform) => invoke<void>("disconnect", { platform }),
  getStatuses: () => invoke<PlatformStatus[]>("get_statuses"),
  getAppInfo: () => invoke<AppInfo>("get_app_info"),
  setEulerApiKey: (key: string | null) => invoke<void>("set_euler_api_key", { key }),
  setServerPort: (port: number) => invoke<void>("set_server_port", { port }),
  simulateEvent: (kind: SimKind, platform: Platform = "tiktok") => invoke<LiveEvent>("simulate_event", { kind, platform }),
  simulateBurst: (count: number, platform: Platform = "tiktok") => invoke<void>("simulate_burst", { count, platform }),
  recentEvents: (limit: number) => invoke<LiveEvent[]>("recent_events", { limit }),

  // Reglas y cola
  listRules: () => invoke<Rule[]>("list_rules"),
  saveRule: (rule: Rule) => invoke<void>("save_rule", { rule }),
  deleteRule: (id: string) => invoke<boolean>("delete_rule", { id }),
  setRuleEnabled: (id: string, enabled: boolean) => invoke<void>("set_rule_enabled", { id, enabled }),
  testRule: (id: string) => invoke<"queued" | "dropped" | "empty">("test_rule", { id }),
  listActionTypes: () => invoke<string[]>("list_action_types"),
  queueStats: () => invoke<QueueInfo>("queue_stats"),
  clearQueue: () => invoke<void>("clear_queue"),

  // Sonidos y medios
  listSounds: () => invoke<Sound[]>("list_sounds"),
  importSound: (path: string, name?: string) => invoke<Sound>("import_sound", { path, name }),
  updateSound: (id: string, patch: { name?: string; volume?: number }) =>
    invoke<Sound>("update_sound", { id, name: patch.name, volume: patch.volume }),
  deleteSound: (id: string) => invoke<void>("delete_sound", { id }),
  previewSound: (id: string) => invoke<void>("preview_sound", { id }),
  stopAudio: () => invoke<void>("stop_audio"),
  listMedia: () => invoke<Media[]>("list_media"),
  importMedia: (path: string, name?: string) => invoke<Media>("import_media", { path, name }),
  renameMedia: (id: string, name: string) => invoke<void>("rename_media", { id, name }),
  deleteMedia: (id: string) => invoke<void>("delete_media", { id }),

  // TTS
  getTtsConfig: () => invoke<TtsConfig>("get_tts_config"),
  setTtsConfig: (config: TtsConfig) => invoke<TtsConfig>("set_tts_config", { config }),
  listTtsVoices: () => invoke<VoiceInfo[]>("list_tts_voices"),
  getTtsStatus: () => invoke<TtsStatus>("get_tts_status"),
  ttsPreview: (text: string, voice?: string) => invoke<void>("tts_preview", { text, voice }),
  ttsSkip: () => invoke<void>("tts_skip"),
  installPiper: () => invoke<void>("install_piper"),
  installPiperVoice: (id: string) => invoke<void>("install_piper_voice", { id }),
  importPiperVoice: (path: string, name?: string) => invoke<string>("import_piper_voice", { path, name }),
  deletePiperVoice: (name: string) => invoke<void>("delete_piper_voice", { name }),
  listMicDevices: () => invoke<string[]>("list_mic_devices"),
  getMicStatus: () => invoke<MicStatus>("get_mic_status"),

  // Overlays
  listOverlays: () => invoke<OverlayDef[]>("list_overlays"),
  getOverlayConfig: (id: string) => invoke<OverlayConfig>("get_overlay_config", { id }),
  setOverlayConfig: (id: string, patch: Partial<OverlayConfig>) => invoke<OverlayConfig>("set_overlay_config", { id, patch }),
  resetOverlayConfig: (id: string) => invoke<OverlayConfig>("reset_overlay_config", { id }),
  testOverlay: (id: string) => invoke<void>("test_overlay", { id }),

  // Metas, timers, ranking y sesión
  listGoals: () => invoke<Goal[]>("list_goals"),
  saveGoal: (goal: Goal) => invoke<void>("save_goal", { goal }),
  deleteGoal: (id: string) => invoke<boolean>("delete_goal", { id }),
  adjustGoal: (id: string, delta: number) => invoke<void>("adjust_goal", { id, delta }),
  resetGoal: (id: string) => invoke<void>("reset_goal", { id }),
  listTimers: () => invoke<TimerView[]>("list_timers"),
  saveTimer: (config: TimerConfig) => invoke<void>("save_timer", { config }),
  deleteTimer: (id: string) => invoke<boolean>("delete_timer", { id }),
  controlTimer: (id: string, op: "start" | "pause" | "resume" | "reset" | "add", seconds?: number) =>
    invoke<void>("control_timer", { id, op, seconds }),
  getLeaderboard: (scope: LeaderboardScope, limit: number) => invoke<DonorEntry[]>("get_leaderboard", { scope, limit }),
  clearDonorHistory: () => invoke<void>("clear_donor_history"),
  getCounters: () => invoke<Counters>("get_counters"),
  newSession: () => invoke<number>("new_session"),
};

export const onStatus = (cb: (s: PlatformStatus) => void): Promise<UnlistenFn> =>
  listen<PlatformStatus>("connection-status", (e) => cb(e.payload));

export const onEvents = (cb: (events: LiveEvent[]) => void): Promise<UnlistenFn> =>
  listen<LiveEvent[]>("live-events", (e) => cb(e.payload));

export const onRuleFired = (cb: (r: FiredReport) => void): Promise<UnlistenFn> =>
  listen<FiredReport>("rule-fired", (e) => cb(e.payload));

export const onInstallProgress = (cb: (p: InstallProgress) => void): Promise<UnlistenFn> =>
  listen<InstallProgress>("tts-install-progress", (e) => cb(e.payload));

/** Abre el selector de archivos del sistema; devuelve la ruta elegida o `null` si se cancela. */
export async function pickFile(name: string, extensions: string[]): Promise<string | null> {
  const picked = await open({ multiple: false, directory: false, filters: [{ name, extensions }] });
  return typeof picked === "string" ? picked : null;
}

/** Los errores de los comandos llegan como string (AppError serializado). */
export function errorMessage(err: unknown): string {
  return typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
}
