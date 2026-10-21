import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { onUpdateProgress, type UpdateProgress } from "../../lib/system";
import { ProgressBar } from "../ui";

type Stage = "idle" | "preparing" | "downloading" | "installing";

/** Etapas visibles de una actualización: preparando → descargando (con %) → instalando (la app se reinicia sola). */
export function useInstall() {
  const [stage, setStage] = useState<Stage>("idle");
  const [progress, setProgress] = useState<UpdateProgress | null>(null);

  useEffect(() => {
    const un = onUpdateProgress((p) => {
      setProgress(p);
      setStage(p.total && p.downloaded >= p.total ? "installing" : "downloading");
    });
    return () => void un.then((f) => f());
  }, []);

  /** Ejecuta `work` (preparar + instalar) mostrando las etapas; si falla vuelve a «idle» y relanza el error. */
  async function start<T>(work: () => Promise<T>): Promise<T> {
    setProgress(null);
    setStage("preparing");
    try {
      return await work();
    } catch (e) {
      setStage("idle");
      throw e;
    }
  }

  return { stage, progress, start, active: stage !== "idle" };
}

export function InstallStatus({ stage, progress }: { stage: Stage; progress: UpdateProgress | null }) {
  const { t } = useTranslation();
  if (stage === "idle") return null;
  const percent = progress?.total ? (progress.downloaded / progress.total) * 100 : null;
  const mb = (n: number) => (n / 1048576).toFixed(1);
  const label =
    stage === "preparing"
      ? t("system.update.preparing")
      : stage === "installing"
        ? t("system.update.finishing")
        : progress
          ? t("system.update.downloadingMb", { done: mb(progress.downloaded), total: progress.total ? mb(progress.total) : "?", percent: percent === null ? "" : ` (${Math.round(percent)}%)` })
          : t("system.update.preparing");
  return (
    <div className="mt-2 space-y-1">
      <ProgressBar percent={stage === "downloading" ? percent : stage === "installing" ? 100 : null} label={label} />
      <p className="text-[11px] text-zinc-500">{t("system.update.restartNote")}</p>
    </div>
  );
}
