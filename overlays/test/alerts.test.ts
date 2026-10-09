import { describe, expect, it } from "vitest";
import { config, load, overlayMsg } from "./harness";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const alerts = (root: HTMLElement) => [...root.querySelectorAll<HTMLElement>(".alert")];
const cfg = { titleSize: 40, textSize: 30, mediaMaxWidth: 60, animation: "pop" };
const alert = (extra: Record<string, unknown> = {}) => ({ id: "x", title: "Ana", text: "envió una rosa", durationMs: 500, ...extra });

describe("alertas", () => {
  it("muestra título y texto con el tamaño configurado", () => {
    const p = load("alerts");
    p.send(config("alerts", cfg));
    p.send(overlayMsg("alerts", alert()));
    const [a] = alerts(p.root);
    expect(a?.querySelector<HTMLElement>(".title")?.textContent).toBe("Ana");
    expect(a?.querySelector<HTMLElement>(".title")?.style.fontSize).toBe("40px");
    expect(a?.querySelector<HTMLElement>(".text")?.style.fontSize).toBe("30px");
    expect(a?.classList.contains("pop")).toBe(true);
    p.win.close();
  });

  it("muestra las alertas de una en una, en orden, y las retira solas", async () => {
    const p = load("alerts");
    p.send(config("alerts", cfg));
    p.send(overlayMsg("alerts", alert({ title: "Primera" })));
    p.send(overlayMsg("alerts", alert({ title: "Segunda" })));
    expect(alerts(p.root)).toHaveLength(1);
    expect(p.root.textContent).toContain("Primera");
    await sleep(1_300); // 500 ms de duración + 480 ms de salida
    expect(alerts(p.root)).toHaveLength(1);
    expect(p.root.textContent).toContain("Segunda");
    await sleep(1_300);
    expect(alerts(p.root)).toHaveLength(0);
    p.win.close();
  });

  it("ignora mensajes de otros canales y datos vacíos", () => {
    const p = load("alerts");
    p.send(config("alerts", cfg));
    p.send(overlayMsg("goals", alert()));
    p.send(overlayMsg("alerts", null));
    expect(alerts(p.root)).toHaveLength(0);
  });

  it("usa medios locales con token, y vídeo silenciado", () => {
    const p = load("alerts");
    p.send(config("alerts", cfg));
    p.send(overlayMsg("alerts", alert({ media: { kind: "image", url: "/media/abc.gif" } })));
    const img = p.root.querySelector("img.media");
    expect(img?.getAttribute("src")).toBe("/media/abc.gif?token=tok");
    expect((img as HTMLElement).style.maxWidth).toBe("60vw");
    p.win.close();

    const v = load("alerts");
    v.send(config("alerts", cfg));
    v.send(overlayMsg("alerts", alert({ media: { kind: "video", url: "/media/clip.mp4" } })));
    const video = v.root.querySelector("video") as HTMLVideoElement | null;
    expect(video?.getAttribute("src")).toBe("/media/clip.mp4?token=tok");
    expect(video?.muted).toBe(true);
    v.win.close();
  });

  it("SEGURIDAD: rutas de medios peligrosas y URLs no HTTPS no se cargan", () => {
    const p = load("alerts");
    p.send(config("alerts", cfg));
    for (const url of ["/media/../etc/passwd", "http://evil/x.png", "javascript:alert(1)", "//evil/x.png", "/otro/x.png"]) {
      p.send(overlayMsg("alerts", alert({ title: url, media: { kind: "image", url }, imageUrl: url, avatar: url, durationMs: 500 })));
    }
    expect(p.root.querySelectorAll("img, video")).toHaveLength(0);
    p.win.close();
  });

  it("SEGURIDAD: el texto de la alerta nunca es HTML", () => {
    const p = load("alerts");
    p.send(config("alerts", cfg));
    p.send(overlayMsg("alerts", alert({ title: "<img src=x onerror=1>", text: "<script>window.__a=1</script>" })));
    expect(p.root.querySelector("img")).toBeNull();
    expect(p.root.querySelector("script")).toBeNull();
    expect((p.win as unknown as { __a?: number }).__a).toBeUndefined();
    p.win.close();
  });

  it("la duración se acota entre 0,5 s y 60 s", async () => {
    const p = load("alerts");
    p.send(config("alerts", cfg));
    p.send(overlayMsg("alerts", alert({ durationMs: 1 }))); // se sube a 500 ms
    await sleep(1_200);
    expect(alerts(p.root)).toHaveLength(0);
    p.win.close();
  });
});
