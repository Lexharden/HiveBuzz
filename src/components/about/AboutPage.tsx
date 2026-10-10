import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Card } from "../ui";
import { BRAND_ICONS } from "./brandIcons";
import { CREDITS, LINKS } from "./credits";

const open = (url: string) => void openUrl(url).catch(() => undefined);

function BrandIcon({ path }: { path: string }) {
  return (
    <svg viewBox="0 0 24 24" aria-hidden className="h-[18px] w-[18px] fill-current">
      <path d={path} />
    </svg>
  );
}

/** Botón de red social con el color de la marca. */
function SocialBtn({ url, className, icon, children }: { url: string; className: string; icon?: string; children: ReactNode }) {
  return (
    <button
      type="button"
      title={url}
      onClick={() => open(url)}
      className={`inline-flex items-center gap-2 rounded-lg px-3.5 py-2 text-sm font-semibold text-white transition-opacity hover:opacity-90 ${className}`}
    >
      {icon && <BrandIcon path={icon} />}
      {children}
    </button>
  );
}

/** Acerca de: versión, comunidad, quién hace HiveBuzz, equipo y cómo apoyar el proyecto. */
export function AboutPage() {
  const { t } = useTranslation();
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    void getVersion().then(setVersion).catch(() => undefined);
  }, []);
  const team = CREDITS.filter((c) => c.members.length > 0);

  return (
    <div className="min-h-0 flex-1 overflow-y-auto pb-4">
      <div className="max-w-2xl space-y-4">
        <Card>
          <div className="flex items-center gap-4">
            <div className="flex h-24 w-24 flex-none items-center justify-center rounded-2xl border-2 border-amber-400 bg-brand-950">
              <img src="/logo.png" alt="" className="h-20 w-20 drop-shadow-[0_0_10px_rgba(249,74,32,0.45)]" />
            </div>
            <div>
              <h2 className="text-2xl font-bold text-white">{t("app.name")}</h2>
              {version && <p className="mt-1 text-sm text-zinc-400">{t("about.version", { version })}</p>}
            </div>
          </div>
          <p className="mt-4 text-sm text-zinc-300">{t("about.what")}</p>
        </Card>

        <Card title={t("about.community.title")}>
          <p className="mb-3 text-sm text-zinc-300">{t("about.community.body")}</p>
          <div className="flex flex-wrap gap-2">
            <SocialBtn url={LINKS.discord} className="bg-[#5865F2]" icon={BRAND_ICONS.discord}>
              {t("about.community.discord")}
            </SocialBtn>
            <SocialBtn url={LINKS.youtube} className="bg-[#FF0000]" icon={BRAND_ICONS.youtube}>
              {t("about.community.youtube")}
            </SocialBtn>
          </div>
        </Card>

        <Card title={t("about.dev.title")}>
          <p className="text-lg font-semibold text-white">{LINKS.developer}</p>
          <p className="mb-3 mt-1 text-sm text-zinc-300">{t("about.dev.body")}</p>
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              title={LINKS.website}
              onClick={() => open(LINKS.website)}
              className="rounded-lg border border-zinc-600 px-3.5 py-2 text-sm font-semibold text-zinc-100 hover:bg-brand-700/50"
            >
              🌐 {t("about.dev.website")}
            </button>
            <SocialBtn url={LINKS.instagram} className="bg-gradient-to-tr from-[#F58529] via-[#DD2A7B] to-[#8134AF]" icon={BRAND_ICONS.instagram}>
              {LINKS.instagramHandle}
            </SocialBtn>
            <SocialBtn url={LINKS.github} className="bg-[#24292f]" icon={BRAND_ICONS.github}>
              {t("about.dev.github")}
            </SocialBtn>
          </div>
          <p className="mt-2 text-xs text-zinc-500">{LINKS.website.replace("https://", "")}</p>
        </Card>

        {team.length > 0 && (
          <Card title={t("about.team.title")}>
            <p className="mb-3 text-sm text-zinc-300">{t("about.team.body")}</p>
            <div className="space-y-3">
              {team.map((c) => (
                <div key={c.key}>
                  <h3 className="mb-1.5 text-sm font-semibold text-zinc-200">
                    {c.icon} {t(`about.team.cat.${c.key}`)}
                  </h3>
                  <div className="flex flex-wrap gap-2">
                    {c.members.map((m) => {
                      const handle = m.tiktok?.replace(/^@/, "");
                      const label = handle && handle.toLowerCase() !== m.name.toLowerCase() ? `${m.name} · @${handle}` : handle ? `@${handle}` : m.name;
                      return handle ? (
                        <button
                          key={m.name}
                          type="button"
                          title={t("about.team.openTiktok")}
                          onClick={() => open(`https://www.tiktok.com/@${encodeURIComponent(handle)}`)}
                          className="rounded-full border border-zinc-700 bg-brand-950 px-3 py-1 text-xs text-zinc-200 hover:border-amber-400"
                        >
                          {label}
                        </button>
                      ) : (
                        <span key={m.name} className="rounded-full border border-zinc-700 bg-brand-950 px-3 py-1 text-xs text-zinc-200">
                          {label}
                        </span>
                      );
                    })}
                  </div>
                </div>
              ))}
            </div>
          </Card>
        )}

        <Card title={t("about.support.title")}>
          <p className="mb-3 text-sm text-zinc-300">{t("about.support.body")}</p>
          <button
            type="button"
            title={LINKS.kofi}
            onClick={() => open(LINKS.kofi)}
            className="rounded-lg bg-amber-400 px-4 py-2 text-sm font-semibold text-brand-900 hover:opacity-90"
          >
            ☕ {t("about.support.kofi")}
          </button>
          <p className="mt-2 text-xs text-zinc-500">{LINKS.kofi.replace("https://", "")}</p>
        </Card>

        <p className="text-xs text-zinc-500">{t("about.footer")}</p>
      </div>
    </div>
  );
}
