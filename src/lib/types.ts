// El esquema de eventos tiene una única fuente de verdad: el sidecar (espejo de events.rs).
export type { EventType, LiveEvent, LiveGift, LiveUser, Platform } from "../../sidecar/src/types";
import type { Platform } from "../../sidecar/src/types";

export type ConnectionState =
  | "disconnected"
  | "waiting_live"
  | "connected"
  | "signature_error"
  | "reconnecting";

export interface StatusUpdate {
  state: ConnectionState;
  detail: string | null;
  attempt: number | null;
  retryInMs: number | null;
}

/** Estado de una plataforma tal como llega de Rust. */
export interface PlatformStatus extends StatusUpdate {
  platform: Platform;
}

export const PLATFORMS: Platform[] = ["tiktok", "twitch"];

export interface OverlayInfo {
  id: string;
  name: string;
  url: string;
}

export interface AppInfo {
  serverPort: number;
  serverError: string | null;
  overlays: OverlayInfo[];
  mediaBase: string;
  overlayToken: string;
  lastUsername: string | null;
  lastTwitchChannel: string | null;
  hasEulerApiKey: boolean;
}

export type SimKind =
  | "gift"
  | "bigGift"
  | "chat"
  | "like"
  | "follow"
  | "share"
  | "subscribe"
  | "join";

// ---- Reglas (espejo de src-tauri/src/rules/model.rs) ----

export type Trigger =
  | { type: "gift"; giftId?: number; giftName?: string; minCoins?: number }
  | { type: "follow" }
  | { type: "share" }
  | { type: "subscribe" }
  | { type: "join" }
  | { type: "subEmote" }
  | { type: "like"; every: number }
  | { type: "command"; command: string }
  | { type: "keyword"; keywords: string[]; wholeWord: boolean }
  | { type: "goalReached"; goalId: string }
  | { type: "timerEnded"; timerId: string }
  | { type: "api"; name: string };

export type TriggerType = Trigger["type"];

export type Role = "moderator" | "subscriber" | "follower";

export interface Schedule {
  days: number[];
  from: string;
  to: string;
}

export interface Conditions {
  globalCooldownMs: number;
  userCooldownMs: number;
  rolesAny: Role[];
  minTeamLevel?: number;
  minGifterLevel?: number;
  probability: number;
  schedule?: Schedule;
}

/** Acción abierta: `type` + parámetros propios de cada ejecutor. */
export interface ActionSpec {
  type: string;
  [param: string]: unknown;
}

export interface Step {
  delayMs: number;
  action: ActionSpec;
}

export interface Rule {
  id: string;
  name: string;
  enabled: boolean;
  trigger: Trigger;
  conditions: Conditions;
  plan: { mode: "sequence" | "parallel"; steps: Step[] };
  priority?: number;
  ttlMs: number;
  /** Puntos que paga el espectador que la dispara (la regla se vuelve una recompensa canjeable). */
  costPoints?: number;
}

export interface FiredReport {
  ruleId: string;
  ruleName: string;
  ts: number;
  queued: boolean;
}

export interface QueueInfo {
  pending: number;
  running: number;
  jobs: number;
}

// ---- Bibliotecas ----

export interface Sound {
  id: string;
  name: string;
  file: string;
  volume: number;
  createdMs: number;
}

export interface Media {
  id: string;
  name: string;
  file: string;
  kind: "image" | "video";
  createdMs: number;
}

// ---- TTS (espejo de src-tauri/src/tts/policy.rs) ----

export type VoiceMode = "single" | "byRole" | "randomPerUser";

export interface FilterConfig {
  profanityEnabled: boolean;
  profanityMode: "skip" | "censor";
  profanityWords: string[];
  skipLinks: boolean;
  stripEmojis: boolean;
  maxRepeat: number;
  maxChars: number;
}

export interface TtsConfig {
  enabled: boolean;
  command: string | null;
  rolesAny: Role[];
  minTeamLevel: number | null;
  minGifterLevel: number | null;
  recentDonorsMinutes: number | null;
  userCooldownMs: number;
  ignoreUsers: string[];
  ignoreBangCommands: boolean;
  template: string;
  maxWaitMs: number;
  filters: FilterConfig;
  voiceMode: VoiceMode;
  defaultVoice: string | null;
  roleVoices: { moderator: string | null; subscriber: string | null; follower: string | null };
  randomVoices: string[];
  rate: number;
  volume: number;
  piperPath: string | null;
  piperVoicesDir: string | null;
  micGuard: MicGuard;
}

/** Qué hace el TTS cuando el streamer habla: repetir la palabra cortada, repetir el mensaje o saltarlo. */
export type MicGuardMode = "repeatWord" | "repeatMessage" | "skip";

export interface MicGuard {
  enabled: boolean;
  mode: MicGuardMode;
  /** null = micrófono predeterminado. */
  device: string | null;
  thresholdDb: number;
  holdMs: number;
}

export interface MicStatus {
  active: boolean;
  speaking: boolean;
  levelDb: number;
  error: string | null;
}

export interface VoiceInfo {
  id: string;
  engine: string;
  name: string;
  lang: string | null;
}

export interface CatalogEntry {
  id: string;
  label: string;
  approxMb: number;
  installed: boolean;
}

export interface TtsStatus {
  piperInstalled: boolean;
  canInstallPiper: boolean;
  catalog: CatalogEntry[];
  /** Voces de Piper importadas por el usuario (no son del catálogo). */
  custom: VoiceInfo[];
}

export interface InstallProgress {
  stage: string;
  done: number;
  total: number | null;
}

// ---- Overlays (espejo de src-tauri/src/overlay_config/schema.rs) ----

export type FieldGroup = "style" | "behavior";

interface FieldBase {
  key: string;
  /** Clave de i18n. */
  label: string;
  group: FieldGroup;
}

export type FieldDef =
  | (FieldBase & { kind: "color"; default: string })
  | (FieldBase & { kind: "number"; default: number; min: number; max: number; step: number })
  | (FieldBase & { kind: "select"; default: string; options: [string, string][] })
  | (FieldBase & { kind: "bool"; default: boolean })
  | (FieldBase & { kind: "text"; default: string; maxLen: number });

export interface OverlayDef {
  id: string;
  /** Clave de i18n. */
  name: string;
  fields: FieldDef[];
}

export type OverlayConfig = Record<string, string | number | boolean>;

// ---- Metas ----

export type GoalKind =
  | { type: "likes" }
  | { type: "follows" }
  | { type: "shares" }
  | { type: "subscribers" }
  | { type: "coins" }
  | { type: "gift"; giftId?: number; giftName?: string };

export type OnReach = { type: "stop" } | { type: "reset" } | { type: "extend"; add: number };

export interface Goal {
  id: string;
  name: string;
  kind: GoalKind;
  target: number;
  current: number;
  onReach: OnReach;
  resetOnSession: boolean;
  reachedCount: number;
}

// ---- Timers ----

export type ExtensionSource =
  | { type: "coins"; perCoins: number }
  | { type: "likes"; perLikes: number }
  | { type: "follow" }
  | { type: "share" }
  | { type: "subscribe" }
  | { type: "gift"; giftId?: number; giftName?: string };

export interface Extension {
  source: ExtensionSource;
  seconds: number;
}

export interface TimerConfig {
  id: string;
  name: string;
  startSeconds: number;
  maxSeconds?: number;
  extensions: Extension[];
}

export type TimerStatus = "idle" | "running" | "paused" | "ended";

export interface TimerView {
  config: TimerConfig;
  status: TimerStatus;
  remainingMs: number;
}

export type LeaderboardScope = "session" | "day" | "all";

export interface DonorEntry {
  userId: string;
  uniqueId: string;
  nickname: string;
  avatar: string;
  coins: number;
  gifts: number;
}

export interface Counters {
  likes: number;
  viewers: number;
  peakViewers: number;
}
