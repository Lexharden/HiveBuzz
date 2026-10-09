import { useCallback, useState, type ReactNode } from "react";
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

export function Btn({
  variant = "ghost",
  className = "",
  ...rest
}: { variant?: BtnVariant } & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      {...rest}
      className={`rounded-md px-3 py-1.5 text-xs disabled:cursor-not-allowed disabled:opacity-50 ${btnCls[variant]} ${className}`}
    />
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
