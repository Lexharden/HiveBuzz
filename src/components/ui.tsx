import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { errorMessage } from "../lib/api";

const inputCls =
  "w-full rounded-md border border-zinc-700 bg-zinc-950 px-2 py-1.5 text-sm outline-none focus:border-amber-400 disabled:opacity-50";

export function Card({ title, hint, actions, children }: { title?: string; hint?: string; actions?: ReactNode; children: ReactNode }) {
  return (
    <section className="rounded-xl border border-brand-700/70 bg-brand-900 p-4">
      {(title || actions) && (
        <div className="mb-3 flex items-start justify-between gap-3">
          <div>
            {title && <h2 className="text-sm font-semibold text-zinc-300">{title}</h2>}
            {hint && <p className="mt-0.5 text-xs text-zinc-500">{hint}</p>}
          </div>
          {actions}
        </div>
      )}
      {children}
    </section>
  );
}

export function Field({ label, hint, children, className = "" }: { label: string; hint?: string; children: ReactNode; className?: string }) {
  return (
    <label className={`block text-xs ${className}`}>
      <span className="mb-1 block text-zinc-400">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-zinc-500">{hint}</span>}
    </label>
  );
}

export function TextInput({ value, onChange, ...rest }: { value: string; onChange: (v: string) => void } & Omit<React.InputHTMLAttributes<HTMLInputElement>, "value" | "onChange">) {
  return <input {...rest} value={value} onChange={(e) => onChange(e.target.value)} className={inputCls} />;
}

/** Número opcional: vacío = `null`. */
export function NumberInput({
  value,
  onChange,
  min,
  max,
  step,
  placeholder,
}: {
  value: number | null | undefined;
  onChange: (v: number | null) => void;
  min?: number;
  max?: number;
  step?: number;
  placeholder?: string;
}) {
  return (
    <input
      type="number"
      value={value ?? ""}
      min={min}
      max={max}
      step={step}
      placeholder={placeholder}
      onChange={(e) => {
        const raw = e.target.value;
        if (raw === "") return onChange(null);
        const n = Number(raw);
        onChange(Number.isFinite(n) ? n : null);
      }}
      className={inputCls}
    />
  );
}

export function Select<T extends string>({
  value,
  onChange,
  options,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
}) {
  return (
    <select value={value} onChange={(e) => onChange(e.target.value as T)} className={inputCls}>
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

export function Checkbox({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <label className="flex items-center gap-2 text-xs text-zinc-300">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} className="h-4 w-4 accent-amber-400" />
      {label}
    </label>
  );
}

type BtnVariant = "primary" | "ghost" | "danger" | "accent";
const btnCls: Record<BtnVariant, string> = {
  primary: "bg-amber-400 text-brand-900 hover:bg-amber-300 font-semibold",
  ghost: "bg-brand-700 hover:bg-brand-600",
  danger: "bg-rose-600/80 hover:bg-rose-500",
  accent: "bg-ember-500 text-white hover:bg-ember-400 font-semibold",
};

/** Rueda giratoria: hereda el color del texto. */
export function Spinner({ className = "h-3 w-3" }: { className?: string }) {
  return (
    <svg className={`inline-block animate-spin align-[-2px] ${className}`} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="3" opacity="0.25" />
      <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
    </svg>
  );
}

/** Botón. Si `onClick` devuelve una promesa (o se pasa `busy`), muestra una rueda y se bloquea hasta que termine. */
export function Btn({
  variant = "ghost",
  className = "",
  busy = false,
  onClick,
  disabled,
  children,
  ...rest
}: { variant?: BtnVariant; busy?: boolean; onClick?: (e: React.MouseEvent<HTMLButtonElement>) => unknown } & Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  "onClick"
>) {
  const [pending, setPending] = useState(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const working = busy || pending;
  return (
    <button
      type="button"
      {...rest}
      aria-busy={working || undefined}
      disabled={disabled || working}
      onClick={(e) => {
        const result = onClick?.(e);
        if (result instanceof Promise) {
          setPending(true);
          void result.then(
            () => alive.current && setPending(false),
            () => alive.current && setPending(false),
          );
        }
      }}
      className={`rounded-md px-3 py-1.5 text-xs disabled:cursor-not-allowed disabled:opacity-50 ${working ? "cursor-progress" : ""} ${btnCls[variant]} ${className}`}
    >
      {working && <Spinner className="mr-1.5 h-3 w-3" />}
      {children}
    </button>
  );
}

/** Barra de progreso; sin `percent` es indeterminada (animada). */
export function ProgressBar({ percent, label }: { percent?: number | null; label?: string }) {
  const known = typeof percent === "number";
  return (
    <div className="space-y-1">
      <div className="relative h-2 overflow-hidden rounded-full bg-zinc-800" role="progressbar" aria-valuenow={known ? Math.round(percent) : undefined}>
        {known ? (
          <div className="h-full rounded-full bg-amber-400 transition-[width] duration-300" style={{ width: `${Math.min(100, Math.max(0, percent))}%` }} />
        ) : (
          <div className="absolute inset-y-0 w-1/3 animate-[hb-slide_1.2s_ease-in-out_infinite] rounded-full bg-amber-400" />
        )}
      </div>
      {label && <p className="text-xs text-zinc-400">{label}</p>}
    </div>
  );
}

// ---- Avisos flotantes ------------------------------------------------------------------------------

type ToastKind = "ok" | "error" | "info";
interface ToastItem {
  id: number;
  kind: ToastKind;
  text: string;
}
const toastListeners = new Set<(t: ToastItem) => void>();
let toastSeq = 0;

/** Muestra un aviso breve en la esquina (hace falta que `ToastHost` esté montado). */
export function toast(text: string, kind: ToastKind = "ok") {
  const item = { id: ++toastSeq, kind, text };
  toastListeners.forEach((l) => l(item));
}

const toastCls: Record<ToastKind, string> = {
  ok: "border-emerald-500/40 bg-emerald-950 text-emerald-200",
  error: "border-rose-500/40 bg-rose-950 text-rose-200",
  info: "border-amber-400/40 bg-brand-900 text-amber-200",
};

export function ToastHost() {
  const [items, setItems] = useState<ToastItem[]>([]);
  useEffect(() => {
    const on = (item: ToastItem) => {
      setItems((cur) => [...cur.slice(-3), item]);
      setTimeout(() => setItems((cur) => cur.filter((x) => x.id !== item.id)), item.kind === "error" ? 6000 : 3000);
    };
    toastListeners.add(on);
    return () => void toastListeners.delete(on);
  }, []);
  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-50 flex w-72 flex-col gap-2" role="status" aria-live="polite">
      {items.map((i) => (
        <div key={i.id} className={`animate-[hb-pop_0.2s_ease-out] rounded-lg border px-3 py-2 text-xs shadow-lg ${toastCls[i.kind]}`}>
          {i.kind === "ok" ? "✓ " : i.kind === "error" ? "✕ " : ""}
          {i.text}
        </div>
      ))}
    </div>
  );
}

export function ErrorText({ error }: { error: string | null }) {
  return error ? <p className="text-xs text-rose-400">{error}</p> : null;
}

/** Ejecuta una acción asíncrona con estado de ocupado y error visible. */
export function useAction() {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const run = useCallback(async <T,>(fn: () => Promise<T>): Promise<T | undefined> => {
    setError(null);
    setBusy(true);
    try {
      return await fn();
    } catch (e) {
      setError(errorMessage(e));
      return undefined;
    } finally {
      setBusy(false);
    }
  }, []);
  return { run, error, busy, clear: () => setError(null) };
}
