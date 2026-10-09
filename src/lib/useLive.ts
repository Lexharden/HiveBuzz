import { useCallback, useEffect, useRef, useState } from "react";
import { api, onEvents, onStatus } from "./api";
import type { AppInfo, LiveEvent, Platform, StatusUpdate } from "./types";

/** Máximo de eventos que se conservan en memoria en la UI. */
export const MAX_UI_EVENTS = 300;

const DISCONNECTED: StatusUpdate = {
  state: "disconnected",
  detail: null,
  attempt: null,
  retryInMs: null,
};

export const INITIAL_STATUSES: Record<Platform, StatusUpdate> = { tiktok: DISCONNECTED, twitch: DISCONNECTED };

/** Agrega eventos nuevos al final sin pasar de `max`, sin duplicar ids. */
export function appendEvents(prev: LiveEvent[], incoming: LiveEvent[], max = MAX_UI_EVENTS): LiveEvent[] {
  if (incoming.length === 0) return prev;
  const seen = new Set(prev.map((e) => e.id));
  const fresh = incoming.filter((e) => !seen.has(e.id));
  if (fresh.length === 0) return prev;
  const merged = prev.concat(fresh);
  return merged.length > max ? merged.slice(merged.length - max) : merged;
}

export function useLive() {
  const [statuses, setStatuses] = useState<Record<Platform, StatusUpdate>>(INITIAL_STATUSES);
  const [events, setEvents] = useState<LiveEvent[]>([]);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const cancelled = useRef(false);

  const refreshInfo = useCallback(async () => {
    try {
      setInfo(await api.getAppInfo());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    cancelled.current = false;
    const unlisten: (() => void)[] = [];

    void (async () => {
      // Primero se escucha y después se consulta el estado, para no perder cambios entre ambos.
      const offStatus = await onStatus(({ platform, ...status }) => setStatuses((prev) => ({ ...prev, [platform]: status })));
      const offEvents = await onEvents((batch) => setEvents((prev) => appendEvents(prev, batch)));
      if (cancelled.current) {
        offStatus();
        offEvents();
        return;
      }
      unlisten.push(offStatus, offEvents);

      try {
        const all = await api.getStatuses();
        setStatuses((prev) => {
          const next = { ...prev };
          for (const { platform, ...status } of all) next[platform] = status;
          return next;
        });
        const recent = await api.recentEvents(100);
        setEvents((prev) => appendEvents(recent, prev));
      } catch (e) {
        setError(String(e));
      }
      await refreshInfo();
    })();

    return () => {
      cancelled.current = true;
      unlisten.forEach((f) => f());
    };
  }, [refreshInfo]);

  const clearEvents = useCallback(() => setEvents([]), []);

  return { statuses, events, info, error, refreshInfo, clearEvents };
}
