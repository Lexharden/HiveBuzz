import { describe, expect, it } from "vitest";
import { config, load, overlayMsg } from "./harness";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const q = <T extends Element>(root: ParentNode, sel: string) => [...root.querySelectorAll<T>(sel)];

describe("encuesta", () => {
  const cfg = { width: 380, trackColor: "#333333", barHeight: 20, showVotes: true, showPercent: true, showTimer: true, hideAfterSec: 15 };
  const poll = (over: Record<string, unknown> = {}) => ({
    kind: "poll",
    poll: {
      id: 1,
      question: "¿Qué jugamos?",
      options: [
        { label: "Minecraft", votes: 6 },
        { label: "Fortnite", votes: 2 },
      ],
      total: 8,
      endsAtMs: Date.now() + 90_000,
      ended: false,
      winners: [],
      ...over,
    },
  });

  it("muestra pregunta, opciones, votos, porcentajes y cuenta atrás", () => {
    const p = load("poll");
    p.send(config("poll", cfg));
    p.send(overlayMsg("poll", poll()));
    expect(p.root.querySelector(".question")?.textContent).toBe("¿Qué jugamos?");
    const rows = q<HTMLElement>(p.root, ".row");
    expect(rows).toHaveLength(2);
    expect(rows[0]?.textContent).toContain("1. Minecraft");
    expect(rows[0]?.textContent).toContain("6 · 75%");
    expect(rows[1]?.textContent).toContain("2 · 25%");
    expect(rows[0]?.querySelector<HTMLElement>(".fill")?.style.width).toBe("75%");
    expect(p.root.querySelector(".timer")?.textContent).toMatch(/^1:[23]\d$/);
    expect(p.root.querySelector(".total")?.textContent).toContain("8 votos");
  });

  it("sin votos todo está en 0% y no divide entre cero", () => {
    const p = load("poll");
    p.send(config("poll", cfg));
    p.send(overlayMsg("poll", poll({ options: [{ label: "A", votes: 0 }, { label: "B", votes: 0 }], total: 0 })));
    expect(q<HTMLElement>(p.root, ".fill").map((f) => f.style.width)).toEqual(["0%", "0%"]);
  });

  it("al terminar marca a los ganadores y apaga el resto", () => {
    const p = load("poll");
    p.send(config("poll", cfg));
    p.send(overlayMsg("poll", poll({ ended: true, winners: [0] })));
    const rows = q<HTMLElement>(p.root, ".row");
    expect(rows[0]?.classList.contains("winner")).toBe(true);
    expect(rows[1]?.classList.contains("loser")).toBe(true);
    expect(p.root.querySelector(".timer")?.textContent).toBe("Finalizada");
  });

  it("un empate marca a ambos como ganadores", () => {
    const p = load("poll");
    p.send(config("poll", cfg));
    p.send(overlayMsg("poll", poll({ ended: true, winners: [0, 1] })));
    expect(q(p.root, ".winner")).toHaveLength(2);
  });

  it("respeta las opciones de mostrar votos, porcentaje y timer", () => {
    const p = load("poll");
    p.send(config("poll", { ...cfg, showVotes: false, showPercent: false, showTimer: false }));
    p.send(overlayMsg("poll", poll()));
    expect(q<HTMLElement>(p.root, ".nums").map((n) => n.textContent)).toEqual(["", ""]);
    expect(p.root.querySelector(".foot")).toBeNull();
  });

  it("se oculta pasado el tiempo configurado desde que terminó, y 'none' la quita", async () => {
    const p = load("poll");
    p.send(config("poll", { ...cfg, hideAfterSec: 0.05 }));
    p.send(overlayMsg("poll", poll({ ended: true, winners: [0] })));
    expect(p.root.querySelector<HTMLElement>(".poll")?.style.display).toBe("");
    await sleep(90);
    p.send(config("poll", { ...cfg, hideAfterSec: 0.05 })); // fuerza el repintado sin esperar al reloj interno
    expect(p.root.querySelector<HTMLElement>(".poll")?.style.display).toBe("none");

    const p2 = load("poll");
    p2.send(config("poll", cfg));
    p2.send(overlayMsg("poll", poll()));
    p2.send(overlayMsg("poll", { kind: "none" }));
    expect(p2.root.querySelector<HTMLElement>(".poll")?.style.display).toBe("none");
  });

  it("hideAfterSec=0 la deja visible siempre", async () => {
    const p = load("poll");
    p.send(config("poll", { ...cfg, hideAfterSec: 0 }));
    p.send(overlayMsg("poll", poll({ ended: true, winners: [0] })));
    await sleep(60);
    p.send(config("poll", { ...cfg, hideAfterSec: 0 }));
    expect(p.root.querySelector<HTMLElement>(".poll")?.style.display).toBe("");
  });

  it("SEGURIDAD: pregunta y opciones con HTML se muestran como texto", () => {
    const p = load("poll");
    p.send(config("poll", cfg));
    p.send(overlayMsg("poll", poll({ question: '<img src=x onerror="window.__p=1">', options: [{ label: "<b>x</b>", votes: 1 }, { label: "y", votes: 0 }], total: 1 })));
    expect(p.root.querySelectorAll("img, b")).toHaveLength(0);
    expect(p.root.textContent).toContain("<img src=x");
    expect((p.win as unknown as { __p?: number }).__p).toBeUndefined();
  });

  it("mensajes malformados no la rompen", () => {
    const p = load("poll");
    p.send(config("poll", cfg));
    p.send(overlayMsg("poll", { kind: "poll", poll: { options: "no" } }));
    p.send(overlayMsg("poll", null));
    p.sendRaw("{no es json");
    p.send(overlayMsg("poll", poll()));
    expect(q(p.root, ".row")).toHaveLength(2);
  });
});

describe("ruleta", () => {
  const cfg = { size: 420, pointerColor: "#ffffff", showWhenIdle: false, resultSec: 6 };
  const segs = (n: number) => Array.from({ length: n }, (_, i) => ({ label: `Premio ${i}`, color: i % 2 ? "#ef4444" : "#3b82f6", weight: 1 }));
  const wheelEl = (root: HTMLElement) => root.querySelector<HTMLElement>(".wheel");
  const angle = (root: HTMLElement) => {
    const t = root.querySelector<SVGGElement>(".disc")?.style.transform ?? "";
    const m = /rotate\((-?[\d.]+)deg\)/.exec(t);
    return m ? Number(m[1]) : NaN;
  };

  it("en reposo está oculta por defecto y visible si se pide", () => {
    const p = load("wheel");
    p.send(config("wheel", cfg));
    p.send(overlayMsg("wheel", { kind: "idle", segments: segs(4) }));
    expect(wheelEl(p.root)?.style.display).toBe("none");
    p.send(config("wheel", { ...cfg, showWhenIdle: true }));
    expect(wheelEl(p.root)?.style.display).toBe("");
    expect(q(p.root, ".disc path")).toHaveLength(4);
    expect(q(p.root, ".seg-label").map((t) => t.textContent)).toEqual(["Premio 0", "Premio 1", "Premio 2", "Premio 3"]);
  });

  it("al girar aparece, dibuja un gajo por premio y termina con el ganador bajo el puntero", () => {
    for (const n of [2, 3, 5, 8, 24]) {
      for (const winner of [0, n - 1, Math.floor(n / 2)]) {
        const p = load("wheel");
        p.send(config("wheel", cfg));
        p.send(overlayMsg("wheel", { kind: "spin", id: 1, segments: segs(n), winner, durationMs: 600, user: "Ana" }));
        expect(wheelEl(p.root)?.style.display).toBe("");
        expect(q(p.root, ".disc path")).toHaveLength(n);
        const deg = angle(p.root);
        const underPointer = (((-deg % 360) + 360) % 360); // grados (horario desde arriba) del disco que quedan arriba
        expect(Math.floor(underPointer / (360 / n)), `n=${n} winner=${winner} deg=${deg}`).toBe(winner);
        expect(deg).toBeGreaterThan(360 * 5); // da varias vueltas
      }
    }
  });

  it("muestra el nombre del ganador y de quién la giró al terminar, y luego se esconde", async () => {
    const p = load("wheel");
    p.send(config("wheel", { ...cfg, resultSec: 1 }));
    p.send(overlayMsg("wheel", { kind: "spin", id: 1, segments: segs(3), winner: 1, durationMs: 500, user: "Ana" }));
    const box = p.root.querySelector<HTMLElement>(".winner");
    expect(box?.style.display).toBe("none");
    await sleep(560);
    expect(box?.style.display).toBe("");
    expect(box?.textContent).toContain("Premio 1");
    expect(box?.textContent).toContain("Ana");
    await sleep(1_050);
    expect(box?.style.display).toBe("none");
    expect(wheelEl(p.root)?.style.display).toBe("none");
  });

  it("durante un giro ignora los mensajes en reposo (no cambia los premios a mitad)", () => {
    const p = load("wheel");
    p.send(config("wheel", cfg));
    p.send(overlayMsg("wheel", { kind: "spin", id: 1, segments: segs(3), winner: 0, durationMs: 600, user: "" }));
    p.send(overlayMsg("wheel", { kind: "idle", segments: segs(6) }));
    expect(q(p.root, ".disc path")).toHaveLength(3);
  });

  it("un premio, ninguno o datos raros no rompen el dibujo", () => {
    const p = load("wheel");
    p.send(config("wheel", { ...cfg, showWhenIdle: true }));
    p.send(overlayMsg("wheel", { kind: "idle", segments: [] }));
    expect(wheelEl(p.root)?.style.display).toBe("none");
    p.send(overlayMsg("wheel", { kind: "idle", segments: segs(1) }));
    expect(q(p.root, ".disc circle").length).toBeGreaterThan(0);
    p.send(overlayMsg("wheel", { kind: "spin", id: 2, segments: "no", winner: 0 }));
    p.send(overlayMsg("wheel", { kind: "spin", id: 3, segments: segs(3), winner: 99, durationMs: "x" }));
    expect(q(p.root, ".disc path")).toHaveLength(3);
    p.send(overlayMsg("wheel", null));
  });

  it("SEGURIDAD: etiquetas con HTML o nombres raros se pintan como texto", () => {
    const p = load("wheel");
    p.send(config("wheel", { ...cfg, showWhenIdle: true }));
    const evil = [{ label: '<img src=x onerror="window.__p=1">', color: "#ff0000", weight: 1 }, { label: "ok", color: "#00ff00", weight: 1 }];
    p.send(overlayMsg("wheel", { kind: "idle", segments: evil }));
    expect(p.root.querySelectorAll("img")).toHaveLength(0);
    expect(q(p.root, ".seg-label")[0]?.textContent).toContain("<img src=x");
    p.send(overlayMsg("wheel", { kind: "spin", id: 1, segments: evil, winner: 0, durationMs: 500, user: "<b>x</b>" }));
    expect(p.root.querySelectorAll("b")).toHaveLength(0);
    expect((p.win as unknown as { __p?: number }).__p).toBeUndefined();
  });
});

describe("sonando ahora", () => {
  const cfg = { width: 420, coverSize: 72, showCover: true, showProgress: true, showRequester: true, hideWhenPaused: false };
  const np = (over: Record<string, unknown> = {}) => ({
    kind: "nowplaying",
    playing: true,
    title: "La Bamba",
    artist: "Ritchie Valens",
    image: "https://i.scdn.co/image/abc",
    progressMs: 30_000,
    durationMs: 120_000,
    requestedBy: "ana",
    ...over,
  });

  it("muestra título, artista, carátula, quién la pidió y el progreso", () => {
    const p = load("nowplaying");
    p.send(config("nowplaying", cfg));
    p.send(overlayMsg("nowplaying", np()));
    expect(p.root.querySelector(".title")?.textContent).toBe("La Bamba");
    expect(p.root.querySelector(".artist")?.textContent).toBe("Ritchie Valens");
    expect(p.root.querySelector(".label")?.textContent).toBe("Sonando ahora");
    expect(p.root.querySelector(".by")?.textContent).toBe("Pedida por @ana");
    expect(p.root.querySelector<HTMLImageElement>("img.cover")?.src).toBe("https://i.scdn.co/image/abc");
    expect(p.root.querySelector<HTMLElement>(".bar > i")?.style.width).toBe("25%");
  });

  it("no inserta HTML de títulos ni usa carátulas que no sean https", () => {
    const p = load("nowplaying");
    p.send(config("nowplaying", cfg));
    p.send(overlayMsg("nowplaying", np({ title: "<img src=x onerror=alert(1)>", artist: "<b>x</b>", image: "http://inseguro/x.jpg", requestedBy: "<i>y</i>" })));
    expect(p.root.querySelector(".title")?.textContent).toBe("<img src=x onerror=alert(1)>");
    expect(p.root.querySelectorAll(".title img, .artist b, .by i")).toHaveLength(0);
    expect(p.root.querySelector("img.cover")).toBeNull();
  });

  it("sin canción se oculta y al llegar «none» desaparece", () => {
    const p = load("nowplaying");
    p.send(config("nowplaying", cfg));
    const card = () => p.root.querySelector<HTMLElement>(".np");
    expect(card()?.style.display).toBe("none");
    p.send(overlayMsg("nowplaying", np()));
    expect(card()?.style.display).toBe("");
    p.send(overlayMsg("nowplaying", { kind: "none" }));
    expect(card()?.style.display).toBe("none");
  });

  it("en pausa muestra «En pausa» o se oculta si así se configura", () => {
    const p = load("nowplaying");
    p.send(config("nowplaying", cfg));
    p.send(overlayMsg("nowplaying", np({ playing: false })));
    expect(p.root.querySelector(".label")?.textContent).toBe("En pausa");
    p.send(config("nowplaying", { ...cfg, hideWhenPaused: true }));
    expect(p.root.querySelector<HTMLElement>(".np")?.style.display).toBe("none");
  });

  it("respeta las opciones de mostrar y no divide entre cero", () => {
    const p = load("nowplaying");
    p.send(config("nowplaying", { ...cfg, showCover: false, showProgress: false, showRequester: false }));
    p.send(overlayMsg("nowplaying", np({ durationMs: 0 })));
    expect(p.root.querySelector("img.cover")).toBeNull();
    expect(p.root.querySelector(".bar")).toBeNull();
    expect(p.root.querySelector(".by")).toBeNull();
    p.send(config("nowplaying", cfg));
    expect(p.root.querySelector<HTMLElement>(".bar > i")?.style.width).toBe("0%");
  });

  it("ignora mensajes de otros canales y mal formados", () => {
    const p = load("nowplaying");
    p.send(config("nowplaying", cfg));
    p.send(overlayMsg("poll", np()));
    p.send(overlayMsg("nowplaying", { kind: "nowplaying" }));
    expect(p.root.querySelector<HTMLElement>(".np")?.style.display).toBe("none");
  });
});

describe("idioma de los overlays", () => {
  it("el texto de la encuesta sale en inglés con ?lang=en y en español por defecto", () => {
    const cfg = { width: 380, trackColor: "#333333", barHeight: 20, showVotes: true, showPercent: true, showTimer: true, hideAfterSec: 15 };
    const poll = { kind: "poll", poll: { id: 1, question: "Q", options: [{ label: "A", votes: 1 }, { label: "B", votes: 0 }], total: 1, endsAtMs: Date.now() + 5000, ended: true, winners: [0] } };
    const en = load("poll", { query: "&lang=en" });
    en.send(config("poll", cfg));
    en.send(overlayMsg("poll", poll));
    expect(en.root.querySelector(".total")?.textContent).toBe("1 votes");
    expect(en.root.querySelector(".timer")?.textContent).toBe("Finished");
    const es = load("poll");
    es.send(config("poll", cfg));
    es.send(overlayMsg("poll", poll));
    expect(es.root.querySelector(".total")?.textContent).toBe("1 votos");
    expect(es.root.querySelector(".timer")?.textContent).toBe("Finalizada");
  });

  it("los textos de «sonando ahora» y de eventos tienen versión en inglés", () => {
    const p = load("nowplaying", { query: "&lang=en" });
    p.send(config("nowplaying", { width: 420, coverSize: 72, showCover: false, showProgress: false, showRequester: true, hideWhenPaused: false }));
    p.send(overlayMsg("nowplaying", { kind: "nowplaying", playing: true, title: "T", artist: "A", image: null, progressMs: 0, durationMs: 1000, requestedBy: "ana" }));
    expect(p.root.querySelector(".label")?.textContent).toBe("Now playing");
    expect(p.root.querySelector(".by")?.textContent).toBe("Requested by @ana");
  });
});
