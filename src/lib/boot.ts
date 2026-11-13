import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Estado del arranque en segundo plano (espejo de `BootStatus` en src-tauri/src/boot.rs). */
export type BootStatus = { state: "starting" } | { state: "ready" } | { state: "failed"; error: string };

export const appStatus = () => invoke<BootStatus>("app_status");

export const onBootStatus = (cb: (s: BootStatus) => void): Promise<UnlistenFn> =>
  listen<BootStatus>("boot-status", (e) => cb(e.payload));

/** El arranque solo avanza: una respuesta tardía de «starting» no deshace un «ready» o «failed» ya recibido. */
export function advance(prev: BootStatus, next: BootStatus): BootStatus {
  return prev.state === "starting" ? next : prev;
}
