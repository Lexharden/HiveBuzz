/** Error de un comando tal como lo envía el backend (espejo de `Wire` en src-tauri/src/error.rs). */
export interface AppError {
  kind: "sidecar" | "db" | "migrate" | "keyring" | "json" | "io" | "server" | "invalid";
  /** Texto completo en español (el mismo del registro). */
  message: string;
  /** El texto sin el prefijo del tipo (solo en errores que no son `invalid`). */
  detail?: string;
  /** Código traducible: `errors.code.<code>`. */
  code?: string;
  params?: Record<string, string>;
}

export function isAppError(err: unknown): err is AppError {
  return typeof err === "object" && err !== null && typeof (err as AppError).kind === "string" && typeof (err as AppError).message === "string";
}

/** Lo justo de i18next para traducir (así se puede probar sin montar i18n). */
export interface Translator {
  t: (key: string, params?: Record<string, string>) => string;
  exists: (key: string) => boolean;
}

/**
 * Texto para la UI: el error entero traducido si tiene código; si no, el tipo traducido con el detalle; y si no
 * hay traducción, el texto original en español.
 */
export function describeError(err: AppError, tr: Translator): string {
  if (err.code && tr.exists(`errors.code.${err.code}`)) return tr.t(`errors.code.${err.code}`, err.params ?? {});
  if (err.kind !== "invalid" && err.detail !== undefined && tr.exists(`errors.kind.${err.kind}`)) {
    return tr.t(`errors.kind.${err.kind}`, { detail: err.detail });
  }
  return err.message;
}
