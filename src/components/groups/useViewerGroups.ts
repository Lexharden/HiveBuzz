import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";
import type { ViewerGroup } from "../../lib/types";

/** Grupos de espectadores (VIPs, Amigos…) para elegirlos en reglas y comandos. */
export function useViewerGroups() {
  const [groups, setGroups] = useState<ViewerGroup[]>([]);
  const [loaded, setLoaded] = useState(false);

  const reload = useCallback(async () => {
    try {
      setGroups(await api.listViewerGroups());
    } catch {
      // sin grupos: las reglas con grupos fallan cerradas, y aquí solo se pierde la lista
    } finally {
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  return { groups, loaded, reload };
}
