import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage, pickFile } from "../../lib/api";
import type { FontFamily } from "../../lib/types";
import { Btn, Card, ErrorText, TextInput } from "../ui";

const FONT_EXT = ["woff2", "woff", "ttf", "otf"];

/** Biblioteca de fuentes para los overlays: se descargan de Google Fonts a disco o se suben las propias. */
export function FontsCard() {
  const { t } = useTranslation();
  const [fonts, setFonts] = useState<FontFamily[]>([]);
  const [family, setFamily] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      setFonts(await api.listFonts());
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);
  useEffect(() => {
    void reload();
  }, [reload]);

  async function act(fn: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    try {
      await fn();
      await reload();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card
      title={t("fonts.title")}
      hint={t("fonts.hint")}
      actions={
        <Btn
          disabled={busy}
          onClick={() =>
            void act(async () => {
              const path = await pickFile(t("fonts.files"), FONT_EXT);
              if (path) await api.importFont(path);
            })
          }
        >
          + {t("fonts.upload")}
        </Btn>
      }
    >
      <div className="flex items-end gap-2">
        <div className="min-w-0 flex-1">
          <TextInput value={family} onChange={setFamily} placeholder={t("fonts.googlePlaceholder")} />
        </div>
        <Btn
          variant="primary"
          disabled={busy || family.trim() === ""}
          onClick={() =>
            void act(async () => {
              await api.downloadGoogleFont(family.trim());
              setFamily("");
            })
          }
        >
          {busy ? t("fonts.downloading") : t("fonts.download")}
        </Btn>
      </div>
      <p className="mt-1 text-xs text-zinc-500">
        {t("fonts.googleHint")}{" "}
        <a className="text-amber-300 underline" href="https://fonts.google.com" target="_blank" rel="noreferrer">
          fonts.google.com
        </a>
      </p>
      <ErrorText error={error} />
      {fonts.length === 0 ? (
        <p className="mt-3 text-sm text-zinc-500">{t("fonts.empty")}</p>
      ) : (
        <ul className="mt-3 grid grid-cols-2 gap-2 md:grid-cols-3">
          {fonts.map((f) => (
            <li key={f.family} className="flex items-center gap-2 rounded-lg border border-zinc-800 bg-zinc-950/50 p-2">
              <span className="min-w-0 flex-1 truncate text-sm" title={f.family}>
                {f.family} <span className="text-xs text-zinc-500">{f.weights.join(" · ")}</span>
              </span>
              <Btn
                variant="danger"
                disabled={busy}
                onClick={() => window.confirm(t("fonts.confirmDelete", { name: f.family })) && void act(() => api.deleteFont(f.family))}
              >
                ✕
              </Btn>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}
