import { describe, expect, it } from "vitest";
import { chat, config, eventMsg, FakeSocket, historyMsg, load } from "./harness";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe("núcleo común", () => {
  it("abre el WebSocket con el token de la URL", () => {
    const p = load("feed");
    expect(p.socket.url).toBe("ws://127.0.0.1:17890/ws?token=tok");
  });

  it("aplica la configuración como variables CSS y posición", () => {
    const p = load("feed");
    p.send(
      config("feed", {
        fontFamily: "Verdana",
        fontSize: 22,
        textColor: "#112233",
        accentColor: "#ff0000",
        backgroundColor: "#000000",
        backgroundOpacity: 50,
        borderRadius: 4,
        margin: 30,
        scale: 150,
        anchor: "top-right",
        maxItems: 5,
      }),
    );
    const style = p.doc.documentElement.style;
    expect(style.getPropertyValue("--hb-font")).toContain("Verdana");
    expect(style.getPropertyValue("--hb-size")).toBe("22px");
    expect(style.getPropertyValue("--hb-fg")).toBe("#112233");
    expect(style.getPropertyValue("--hb-accent")).toBe("#ff0000");
    expect(style.getPropertyValue("--hb-bg")).toBe("rgba(0,0,0,0.5)");
    expect(style.getPropertyValue("--hb-radius")).toBe("4px");
    expect(style.getPropertyValue("--hb-margin")).toBe("30px");
    expect(style.getPropertyValue("--hb-scale")).toBe("1.5");
    expect(p.doc.body.getAttribute("data-anchor")).toBe("top-right");
  });

  it("solo hace caso a la configuración de su propio overlay", () => {
    const p = load("feed");
    p.send(config("chat", { fontSize: 99, anchor: "center" }));
    expect(p.doc.documentElement.style.getPropertyValue("--hb-size")).toBe("");
    expect(p.doc.body.getAttribute("data-anchor")).toBe("bottom-left");
  });

  it("un mensaje ilegible o raro no tumba el overlay", () => {
    const p = load("chat");
    p.sendRaw("esto no es json");
    p.sendRaw("null");
    p.sendRaw('{"type":"event"}');
    p.sendRaw('{"type":"event","event":{"type":"chat"}}');
    p.sendRaw('{"type":"history","events":"no-es-array"}');
    p.send(eventMsg(chat("sigo vivo")));
    expect(p.root.textContent).toContain("sigo vivo");
  });

  it("muestra el punto rojo sin conexión y reconecta con espera creciente", async () => {
    const p = load("feed");
    const dot = p.doc.getElementById("hb-dot");
    expect(dot?.classList.contains("show")).toBe(false);
    p.socket.close();
    expect(dot?.classList.contains("show")).toBe(true);
    expect(FakeSocket.instances).toHaveLength(1);
    await sleep(1_100); // primera espera: 1 s
    expect(FakeSocket.instances).toHaveLength(2);
    FakeSocket.instances[1]?.onopen?.();
    expect(dot?.classList.contains("show")).toBe(false);
    p.win.close();
  });

  it("el historial se entrega como repetición (sin animación de entrada)", () => {
    const p = load("chat");
    p.send(config("chat", { maxMessages: 10, hideCommands: true, showAvatar: false }));
    p.send(historyMsg([chat("viejo"), chat("viejo 2")]));
    p.send(eventMsg(chat("nuevo")));
    const cards = [...p.root.querySelectorAll<HTMLElement>(".hb-card")];
    expect(cards).toHaveLength(3);
    expect(cards[0]?.style.animation).toBe("none");
    expect(cards[2]?.style.animation).toBe("");
  });

  it("modo vista previa: añade la cuadrícula de fondo", () => {
    const p = load("feed", { query: "&preview=1" });
    expect(p.doc.body.classList.contains("hb-preview")).toBe(true);
  });

  it("el idioma por defecto es español y un idioma desconocido cae en español", () => {
    const p = load("feed");
    const HB = (p.win as unknown as { HB: { t: (k: string, v?: Record<string, unknown>) => string } }).HB;
    expect(HB.t("ev.gift", { count: 3, name: "Rose" })).toBe("envió 3× Rose");
    expect(HB.t("clave.inexistente")).toBe("clave.inexistente");
    const q = load("feed", { query: "&lang=xx" });
    const HB2 = (q.win as unknown as { HB: { t: (k: string) => string } }).HB;
    expect(HB2.t("ev.follow")).toBe("empezó a seguirte");
  });
});

describe("seguridad de las utilidades", () => {
  it("solo acepta imágenes HTTPS y medios locales con nombre seguro", () => {
    const p = load("feed");
    const HB = (p.win as unknown as { HB: Record<string, (u: unknown) => unknown> }).HB;
    expect(HB.isHttps?.("https://x.com/a.png")).toBe(true);
    for (const bad of ["http://x.com/a.png", "javascript:alert(1)", "data:text/html,x", "//x.com/a.png", "", null, 5]) {
      expect(HB.isHttps?.(bad), String(bad)).toBe(false);
    }
    expect(HB.isLocalMedia?.("/media/abc-123.png")).toBe(true);
    for (const bad of ["/media/../secret", "/media/a/b.png", "http://evil/media/a.png", "/other/a.png", "/media/"]) {
      expect(HB.isLocalMedia?.(bad), bad).toBe(false);
    }
  });

  it("HB.img descarta URLs no HTTPS y se quita sola si la imagen falla", () => {
    const p = load("feed");
    const HB = (p.win as unknown as { HB: { img: (u: string, c: string) => HTMLImageElement | null } }).HB;
    expect(HB.img("javascript:alert(1)", "x")).toBeNull();
    expect(HB.img("http://x.com/a.png", "x")).toBeNull();
    const img = HB.img("https://x.com/a.png", "hb-avatar");
    expect(img?.referrerPolicy).toBe("no-referrer");
    p.root.appendChild(img as HTMLImageElement);
    (img as HTMLImageElement).onerror?.(new p.win.Event("error"));
    expect(p.root.querySelector("img")).toBeNull();
  });
});
