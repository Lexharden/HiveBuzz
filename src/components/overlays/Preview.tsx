import { useEffect, useRef, useState } from "react";

const CANVAS_W = 1920;
const CANVAS_H = 1080;

/**
 * Vista previa en vivo: el propio overlay en un iframe de 1920×1080 reducido para caber.
 * Así se ve el tamaño real que tendrá en OBS, y los cambios de configuración llegan por el mismo
 * WebSocket que usa el overlay de verdad.
 */
export function Preview({ url }: { url: string }) {
  const box = useRef<HTMLDivElement>(null);
  const [scale, setScale] = useState(0.4);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const update = () => setScale(Math.max(0.1, el.clientWidth / CANVAS_W));
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  return (
    <div ref={box} className="w-full overflow-hidden rounded-lg border border-zinc-800 bg-zinc-950" style={{ height: CANVAS_H * scale }}>
      <iframe
        title="preview"
        src={`${url}&preview=1`}
        // Solo scripts del propio overlay; sin acceso a nada más de la app.
        sandbox="allow-scripts allow-same-origin"
        style={{ width: CANVAS_W, height: CANVAS_H, border: 0, transform: `scale(${scale})`, transformOrigin: "top left" }}
      />
    </div>
  );
}
