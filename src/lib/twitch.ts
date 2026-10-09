import { invoke } from "@tauri-apps/api/core";

export interface TwitchAccount {
  login: string;
  userId: string;
}

export interface DeviceInfo {
  userCode: string;
  verificationUri: string;
  expiresInS: number;
}

export type LoginState =
  | { state: "idle" }
  | { state: "pending"; info: DeviceInfo }
  | { state: "done" }
  | { state: "failed"; reason: string };

export interface TwitchStatus {
  loginAvailable: boolean;
  usesBuiltinApp: boolean;
  loggedIn: boolean;
  login: LoginState;
  account: TwitchAccount | null;
}

export interface TwitchConfig {
  clientId: string;
}

export const twitchApi = {
  status: () => invoke<TwitchStatus>("twitch_status"),
  getConfig: () => invoke<TwitchConfig>("twitch_get_config"),
  setConfig: (config: TwitchConfig) => invoke<TwitchConfig>("twitch_set_config", { config }),
  loginStart: () => invoke<DeviceInfo>("twitch_login_start"),
  loginState: () => invoke<LoginState>("twitch_login_state"),
  loginCancel: () => invoke<void>("twitch_login_cancel"),
  logout: () => invoke<void>("twitch_logout"),
  openActivation: () => invoke<void>("twitch_open_activation"),
};
