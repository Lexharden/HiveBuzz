import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage, pickFile } from "../../lib/api";
import type { AppInfo, Media, Sound } from "../../lib/types";
import { Btn, Card, ErrorText } from "../ui";

const AUDIO_EXT = ["mp3", "wav", "ogg", "flac"];
const MEDIA_EXT = ["png", "jpg", "jpeg", "gif", "webp", "mp4", "webm"];

export function LibraryPage({ info }: { info: AppInfo | null }) {
  const { t } = useTranslation();
  const [sounds, setSounds] = useState<Sound[]>([]);
  const [media, setMedia] = useState<Media[]>([]);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);

  const reload = useCallback(async () => {
    try {
      const [s, m] = await Promise.all([api.listSounds(), api.listMedia()]);
      if (!alive.current) return;
      setSounds(s);
      setMedia(m);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    void reload();
    return () => {
      alive.current = false;
      void api.stopAudio().catch(() => undefined);
    };
  }, [reload]);

  async function act(fn: () => Promise<unknown>) {
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <div className="min-h-0 flex-1 space-y-4 overflow-y-auto">
      <Card
        title={t("library.sounds")}
        hint={t("library.soundsHint")}
        actions={
          <Btn
            variant="primary"
            onClick={() =>
              void act(async () => {
                const path = await pickFile(t("library.audioFiles"), AUDIO_EXT);
                if (path) {
                  await api.importSound(path);
                  await reload();
                }
              })
            }
          >
            + {t("library.import")}
          </Btn>
        }
      >
        {sounds.length === 0 && <p className="py-4 text-center text-sm text-zinc-500">{t("library.noSounds")}</p>}
        <ul className="space-y-2">
          {sounds.map((s) => (
            <SoundRow key={s.id} sound={s} onChanged={reload} onError={setError} />
          ))}
        </ul>
      </Card>

      <Card
        title={t("library.media")}
        hint={t("library.mediaHint")}
        actions={
          <Btn
            variant="primary"
            onClick={() =>
              void act(async () => {
                const path = await pickFile(t("library.mediaFiles"), MEDIA_EXT);
                if (path) {
                  await api.importMedia(path);
                  await reload();
                }
              })
            }
          >
            + {t("library.import")}
          </Btn>
        }
      >
        {media.length === 0 && <p className="py-4 text-center text-sm text-zinc-500">{t("library.noMedia")}</p>}
        <ul className="grid grid-cols-2 gap-3 md:grid-cols-3">
          {media.map((m) => (
            <MediaCard key={m.id} media={m} info={info} onChanged={reload} onError={setError} />
          ))}
        </ul>
      </Card>
      <ErrorText error={error} />
    </div>
  );
}

function SoundRow({ sound, onChanged, onError }: { sound: Sound; onChanged: () => Promise<void>; onError: (e: string) => void }) {
  const { t } = useTranslation();
  const [name, setName] = useState(sound.name);
  const [volume, setVolume] = useState(sound.volume);
  const guard = (fn: () => Promise<unknown>) => void fn().catch((e: unknown) => onError(errorMessage(e)));

  return (
    <li className="flex items-center gap-3 rounded-lg border border-zinc-800 bg-zinc-950/50 px-3 py-2">
      <Btn onClick={() => guard(() => api.previewSound(sound.id))} title={t("library.preview")}>▶</Btn>
      <input
        value={name}
        onChange={(e) => setName(e.target.value)}
        onBlur={() => name.trim() !== sound.name && guard(async () => { await api.updateSound(sound.id, { name }); await onChanged(); })}
        className="min-w-0 flex-1 rounded-md border border-transparent bg-transparent px-2 py-1 text-sm hover:border-zinc-700 focus:border-amber-400 focus:outline-none"
      />
      <input
        type="range"
        min={0}
        max={100}
        value={volume}
        onChange={(e) => setVolume(Number(e.target.value))}
        onPointerUp={() => volume !== sound.volume && guard(async () => { await api.updateSound(sound.id, { volume }); await onChanged(); })}
        className="w-32 accent-amber-400"
        title={t("library.volume")}
      />
      <span className="w-10 text-right text-xs text-zinc-400">{volume}%</span>
      <Btn
        variant="danger"
        onClick={() => window.confirm(t("library.confirmDelete", { name: sound.name })) && guard(async () => { await api.deleteSound(sound.id); await onChanged(); })}
      >
        {t("rules.delete")}
      </Btn>
    </li>
  );
}

function MediaCard({ media, info, onChanged, onError }: { media: Media; info: AppInfo | null; onChanged: () => Promise<void>; onError: (e: string) => void }) {
  const { t } = useTranslation();
  const src = info ? `${info.mediaBase}/${media.file}?token=${info.overlayToken}` : undefined;
  const guard = (fn: () => Promise<unknown>) => void fn().catch((e: unknown) => onError(errorMessage(e)));

  return (
    <li className="overflow-hidden rounded-lg border border-zinc-800 bg-zinc-950/50">
      <div className="flex h-28 items-center justify-center bg-zinc-950">
        {src && media.kind === "image" && <img src={src} alt={media.name} className="max-h-full max-w-full object-contain" />}
        {src && media.kind === "video" && <video src={src} muted loop autoPlay playsInline className="max-h-full max-w-full" />}
      </div>
      <div className="flex items-center gap-2 p-2">
        <span className="min-w-0 flex-1 truncate text-xs" title={media.name}>{media.name}</span>
        <Btn
          onClick={() => {
            const name = window.prompt(t("library.rename"), media.name);
            if (name && name.trim()) guard(async () => { await api.renameMedia(media.id, name); await onChanged(); });
          }}
        >
          ✎
        </Btn>
        <Btn
          variant="danger"
          onClick={() => window.confirm(t("library.confirmDelete", { name: media.name })) && guard(async () => { await api.deleteMedia(media.id); await onChanged(); })}
        >
          ✕
        </Btn>
      </div>
    </li>
  );
}
