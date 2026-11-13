import { useEffect, useRef } from "react";
import { onUiChanged, type UiTopic } from "./api";

/**
 * Llama a `refresh` cada vez que el backend avisa de que cambió alguno de `topics`, en lugar de consultar cada
 * segundo. También lo llama una vez al quedar suscrito, por si algo cambió entre la primera carga y la suscripción.
 */
export function useUiChanged(topics: readonly UiTopic[], refresh: () => void) {
  const latest = useRef(refresh);
  useEffect(() => {
    latest.current = refresh;
  });
  const key = topics.join(",");

  useEffect(() => {
    // Local a cada montaje: con StrictMode el efecto se monta dos veces.
    let disposed = false;
    let off: (() => void) | undefined;
    void onUiChanged(key.split(",") as UiTopic[], () => latest.current()).then((unlisten) => {
      if (disposed) {
        unlisten();
        return;
      }
      off = unlisten;
      latest.current();
    });
    return () => {
      disposed = true;
      off?.();
    };
  }, [key]);
}
