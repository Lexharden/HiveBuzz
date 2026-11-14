import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../lib/api";
import { advance, appStatus, onBootStatus, type BootStatus } from "../lib/boot";
import { systemApi } from "../lib/system";

/**
 * Muestra un splash mientras HiveBuzz arranca en segundo plano y monta la app cuando está lista: antes no existe
 * el estado del backend y sus comandos fallarían. Si el arranque falla, enseña el motivo y permite reintentar.
 */
export function Boot({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<BootStatus>({ state: "starting" });

  useEffect(() => {
    let disposed = false;
    let off: (() => void) | undefined;
    // Primero se escucha y después se pregunta, para no perder un aviso que llegue entre ambos.
    void onBootStatus((s) => setStatus((prev) => advance(prev, s))).then((unlisten) => {
      if (disposed) {
        unlisten();
        return;
      }
      off = unlisten;
      void appStatus()
        .then((s) => !disposed && setStatus((prev) => advance(prev, s)))
        .catch((e: unknown) => !disposed && setStatus({ state: "failed", error: errorMessage(e) }));
    });
    return () => {
      disposed = true;
      off?.();
    };
  }, []);

  if (status.state === "ready") return <>{children}</>;

  return (
    <div className="flex h-screen flex-col items-center justify-center gap-4 p-6 text-center">
      <img src="/logo.png" alt="" className={`h-20 w-20 ${status.state === "starting" ? "animate-pulse" : ""}`} />
      <h1 className="text-xl font-bold text-white">{t("app.name")}</h1>
      {status.state === "starting" ? (
        <p className="text-sm text-zinc-400" role="status">
          {t("boot.starting")}
        </p>
      ) : (
        <div role="alert" className="max-w-lg space-y-3">
          <p className="text-sm font-semibold text-rose-300">{t("boot.failed")}</p>
          <pre className="max-h-40 overflow-auto whitespace-pre-wrap rounded-md border border-rose-500/40 bg-rose-500/10 p-3 text-left text-xs text-rose-200">
            {status.error}
          </pre>
          <p className="text-xs text-zinc-400">{t("boot.failedHint")}</p>
          <button
            type="button"
            onClick={() => void systemApi.restartApp()}
            className="rounded-md bg-amber-400 px-4 py-1.5 text-sm font-semibold text-brand-900 hover:bg-amber-300"
          >
            {t("boot.retry")}
          </button>
        </div>
      )}
    </div>
  );
}
