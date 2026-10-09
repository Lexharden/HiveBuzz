import { useTranslation } from "react-i18next";
import type { Platform } from "../lib/types";

const STYLE: Record<Platform, string> = {
  tiktok: "bg-[#fe2c55] text-white",
  twitch: "bg-[#9146ff] text-white",
};

/** Etiqueta con el nombre de la plataforma (el color ayuda, pero el texto siempre está). */
export function PlatformBadge({ platform, className = "" }: { platform: Platform; className?: string }) {
  const { t } = useTranslation();
  return <span className={`inline-block rounded-full px-2 py-0.5 text-[10px] font-bold leading-none ${STYLE[platform]} ${className}`}>{t(`platform.${platform}`)}</span>;
}
