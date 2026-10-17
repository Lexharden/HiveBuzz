import { invoke } from "@tauri-apps/api/core";

export interface ObsConfig {
  host: string;
  port: number;
}

export interface ObsInfo {
  obsVersion: string;
  websocketVersion: string;
}

export interface WebhookReport {
  status: number;
  ok: boolean;
  elapsedMs: number;
  attempts: number;
  body: string;
}

export const integrationsApi = {
  getObsConfig: () => invoke<ObsConfig>("get_obs_config"),
  setObsConfig: (config: ObsConfig) => invoke<ObsConfig>("set_obs_config", { config }),
  hasObsPassword: () => invoke<boolean>("has_obs_password"),
  setObsPassword: (password: string) => invoke<void>("set_obs_password", { password }),
  testObs: () => invoke<ObsInfo>("test_obs"),
  testWebhook: (action: Record<string, unknown>) => invoke<WebhookReport>("test_webhook", { action }),
};
