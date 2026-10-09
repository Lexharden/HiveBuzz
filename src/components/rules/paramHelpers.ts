import type { ActionSpec } from "../../lib/types";

// Lectores tolerantes de los parámetros abiertos de una acción.
export const str = (a: ActionSpec, k: string): string => (typeof a[k] === "string" ? (a[k] as string) : "");
export const num = (a: ActionSpec, k: string): number | null => (typeof a[k] === "number" ? (a[k] as number) : null);
export const bool = (a: ActionSpec, k: string, d: boolean): boolean => (typeof a[k] === "boolean" ? (a[k] as boolean) : d);

/** Devuelve la acción con `key` cambiado; un valor `null`/vacío elimina el parámetro. */
export function set(a: ActionSpec, key: string, value: unknown): ActionSpec {
  const next: ActionSpec = { ...a, [key]: value };
  if (value === null || value === undefined || value === "") delete next[key];
  return next;
}

/** Número, o texto con variables (p. ej. «{coins}»): se guarda como lo escribió el usuario. */
export function amountValue(a: ActionSpec, key: string): string {
  const v = a[key];
  return typeof v === "number" || typeof v === "string" ? String(v) : "";
}

export function toAmount(raw: string): number | string {
  const trimmed = raw.trim();
  return /^-?\d+$/.test(trimmed) ? Number(trimmed) : raw;
}

/** Lista separada por comas ↔ array de textos. */
export const listValue = (a: ActionSpec, k: string): string =>
  Array.isArray(a[k]) ? (a[k] as unknown[]).filter((x) => typeof x === "string").join(", ") : "";
export const toList = (raw: string): string[] =>
  raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
