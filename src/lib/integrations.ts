import { invoke } from "@tauri-apps/api/core";

export interface ObsConfig {
  host: string;
  port: number;
}

export interface ObsInfo {
  obsVersion: string;
  websocketVersion: string;
}

export const integrationsApi = {
  getObsConfig: () => invoke<ObsConfig>("get_obs_config"),
  setObsConfig: (config: ObsConfig) => invoke<ObsConfig>("set_obs_config", { config }),
  hasObsPassword: () => invoke<boolean>("has_obs_password"),
  setObsPassword: (password: string) => invoke<void>("set_obs_password", { password }),
  testObs: () => invoke<ObsInfo>("test_obs"),
};
