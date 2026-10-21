import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../../lib/api";
import { interactApi, type Viewer } from "../../lib/interact";
import type { Member, ViewerGroup } from "../../lib/types";
import { Btn, Card, ErrorText, Field, Select, TextInput, toast } from "../ui";
import { useViewerGroups } from "./useViewerGroups";

type PlatformChoice = "" | "tiktok" | "twitch";

/** Gestión de grupos de espectadores: nombre, usuarios y a qué plataforma aplica cada uno. */
export function ViewerGroupsSection() {
  const { t } = useTranslation();
  const { groups, loaded, reload } = useViewerGroups();
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);

  const guard = async (fn: () => Promise<unknown>) => {
    setError(null);
    try {
      await fn();
      await reload();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <div className="space-y-4">
      <Card title={t("groups.title")} hint={t("groups.hint")}>
        <div className="flex gap-2">
          <TextInput value={name} onChange={setName} placeholder={t("groups.newPlaceholder")} onKeyDown={(e) => e.key === "Enter" && name.trim() && void create()} className="flex-1" />
          <Btn variant="primary" disabled={!name.trim()} onClick={create}>
            + {t("groups.create")}
          </Btn>
        </div>
        <p className="mt-2 text-xs text-zinc-500">{t("groups.renameWarning")}</p>
        <ErrorText error={error} />
      </Card>

      {loaded && groups.length === 0 && <p className="py-4 text-center text-sm text-zinc-500">{t("groups.empty")}</p>}
      {groups.map((g) => (
        <GroupCard key={g.id} group={g} onSave={(next) => guard(() => api.saveViewerGroup(next))} onDelete={() => remove(g)} />
      ))}
    </div>
  );

  async function create() {
    await guard(async () => {
      await api.saveViewerGroup({ id: "", name, members: [] });
      setName("");
      toast(t("groups.created"));
    });
  }

  async function remove(g: ViewerGroup) {
    const uses = await api.viewerGroupUsage(g.id).catch(() => 0);
    const msg = uses > 0 ? t("groups.confirmDeleteUsed", { name: g.name, n: uses }) : t("groups.confirmDelete", { name: g.name });
    if (window.confirm(msg)) await guard(() => api.deleteViewerGroup(g.id));
  }
}

function GroupCard({ group, onSave, onDelete }: { group: ViewerGroup; onSave: (g: ViewerGroup) => Promise<void>; onDelete: () => Promise<void> }) {
  const { t } = useTranslation();
  const [name, setName] = useState(group.name);
  const [text, setText] = useState("");
  const [platform, setPlatform] = useState<PlatformChoice>("");
  const [search, setSearch] = useState("");
  const [found, setFound] = useState<Viewer[]>([]);

  useEffect(() => setName(group.name), [group.name]);

  useEffect(() => {
    if (search.trim().length < 2) {
      setFound([]);
      return;
    }
    const timer = setTimeout(() => void interactApi.listViewers(search.trim(), "lastSeen", 6, 0).then((p) => setFound(p.items)).catch(() => setFound([])), 250);
    return () => clearTimeout(timer);
  }, [search]);

  const has = (handle: string, p?: Member["platform"]) => group.members.some((m) => m.handle === handle.replace(/^@/, "").toLowerCase() && m.platform === p);
  const add = (handles: string[], p: PlatformChoice) => {
    const extra: Member[] = handles
      .map((h) => h.trim().replace(/^@/, "").toLowerCase())
      .filter((h) => h && !has(h, p || undefined))
      .map((handle) => ({ handle, ...(p ? { platform: p } : {}) }));
    if (extra.length === 0) return Promise.resolve();
    return onSave({ ...group, members: [...group.members, ...extra] });
  };

  return (
    <Card
      title={`${group.name} · ${t("groups.count", { n: group.members.length })}`}
      actions={
        <Btn variant="danger" onClick={onDelete}>
          {t("rules.delete")}
        </Btn>
      }
    >
      <div className="space-y-3">
        <Field label={t("groups.name")}>
          <TextInput value={name} onChange={setName} onBlur={() => name.trim() && name.trim() !== group.name && void onSave({ ...group, name })} />
        </Field>

        <div className="flex flex-wrap gap-2">
          {group.members.length === 0 && <span className="text-xs text-zinc-500">{t("groups.noMembers")}</span>}
          {group.members.map((m) => (
            <span key={`${m.platform ?? "*"}:${m.handle}`} className="inline-flex items-center gap-1 rounded-full border border-zinc-700 bg-zinc-950 px-2 py-0.5 text-xs">
              @{m.handle}
              {m.platform && <span className="text-zinc-500">· {m.platform}</span>}
              <button
                type="button"
                aria-label={t("groups.remove")}
                className="text-zinc-500 hover:text-rose-400"
                onClick={() => void onSave({ ...group, members: group.members.filter((x) => x !== m) })}
              >
                ✕
              </button>
            </span>
          ))}
        </div>

        <div className="grid grid-cols-[1fr_9rem_auto] items-end gap-2">
          <Field label={t("groups.add")} hint={t("groups.addHint")}>
            <TextInput value={text} onChange={setText} placeholder="@usuario1, @usuario2" onKeyDown={(e) => e.key === "Enter" && void submit()} />
          </Field>
          <Field label={t("groups.platform")}>
            <Select<PlatformChoice>
              value={platform}
              onChange={setPlatform}
              options={[
                { value: "", label: t("groups.anyPlatform") },
                { value: "tiktok", label: "TikTok" },
                { value: "twitch", label: "Twitch" },
              ]}
            />
          </Field>
          <Btn variant="primary" disabled={!text.trim()} onClick={submit}>
            {t("groups.addBtn")}
          </Btn>
        </div>

        <Field label={t("groups.fromViewers")} hint={t("groups.fromViewersHint")}>
          <TextInput value={search} onChange={setSearch} placeholder={t("groups.searchViewers")} />
        </Field>
        {found.length > 0 && (
          <ul className="flex flex-wrap gap-2">
            {found.map((v) => (
              <li key={v.userId}>
                <Btn onClick={() => add([v.uniqueId], "")} disabled={has(v.uniqueId)}>
                  + @{v.uniqueId.replace(/^@/, "")}
                </Btn>
              </li>
            ))}
          </ul>
        )}
      </div>
    </Card>
  );

  async function submit() {
    await add(text.split(/[\s,;\n]+/), platform);
    setText("");
  }
}
