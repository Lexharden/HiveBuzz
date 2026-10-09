import { useEffect, useState } from "react";

// Banderas pequeñas de la interfaz («ya probé una alerta», «ocultar los primeros pasos»).
// Viven en el navegador de la propia ventana; si no está disponible, la app funciona igual.
const EVENT = "hb-flag";

export function getFlag(key: string): boolean {
  try {
    return window.localStorage.getItem(`hb.${key}`) === "1";
  } catch {
    return false;
  }
}

export function setFlag(key: string, value: boolean): void {
  try {
    window.localStorage.setItem(`hb.${key}`, value ? "1" : "0");
  } catch {
    // Sin almacenamiento: la bandera solo vale hasta cerrar la ventana.
  }
  window.dispatchEvent(new CustomEvent(EVENT, { detail: key }));
}

export function useFlag(key: string): [boolean, (v: boolean) => void] {
  const [value, setValue] = useState(() => getFlag(key));
  useEffect(() => {
    const on = (e: Event) => {
      if ((e as CustomEvent<string>).detail === key) setValue(getFlag(key));
    };
    window.addEventListener(EVENT, on);
    return () => window.removeEventListener(EVENT, on);
  }, [key]);
  return [value, (v) => setFlag(key, v)];
}
