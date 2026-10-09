import { describe, expect, it } from "vitest";
import { config, load, overlayMsg } from "./harness";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const q = <T extends Element>(root: ParentNode, sel: string) => [...root.querySelectorAll<T>(sel)];
const entry = (nickname: string, coins: number, avatar = "") => ({ userId: nickname, uniqueId: nickname.toLowerCase(), nickname, avatar, coins, gifts: 1 });

describe("ranking de donadores", () => {
  const cfg = { width: 320, scope: "session", size: 3, title: "Top", showAvatar: false, showCoins: true, medals: true };
  const board = {
    session: [entry("Ana", 15000), entry("Beto", 900), entry("Cata", 400), entry("Dani", 10)],
    day: [entry("Zoe", 99999)],
    all: [],
  };

  it("muestra el ámbito elegido, limitado al tamaño, con medallas", () => {
    const p = load("leaderboard");
    p.send(config("leaderboard", cfg));
    p.send(overlayMsg("leaderboard", board));
    const rows = q<HTMLElement>(p.root, ".entry");
    expect(rows).toHaveLength(3);
    expect(rows[0]?.textContent).toContain("🥇");
    expect(rows[0]?.textContent).toContain("Ana");
    expect(rows[0]?.textContent).toContain("15.000");
    expect(rows[2]?.textContent).toContain("🥉");
    expect(p.root.textContent).toContain("Top");
    expect(p.root.textContent).not.toContain("Dani");
  });

  it("cambia de ámbito y de estilo en vivo, y sin medallas usa números", () => {
    const p = load("leaderboard");
    p.send(overlayMsg("leaderboard", board));
    p.send(config("leaderboard", { ...cfg, scope: "day", medals: false, showCoins: false }));
    const rows = q<HTMLElement>(p.root, ".entry");
    expect(rows).toHaveLength(1);
    expect(rows[0]?.textContent).toContain("1");
    expect(rows[0]?.textContent).toContain("Zoe");
    expect(rows[0]?.textContent).not.toContain("99");
  });

  it("un ámbito vacío muestra un guion en vez de romperse", () => {
    const p = load("leaderboard");
    p.send(config("leaderboard", { ...cfg, scope: "all" }));
    p.send(overlayMsg("leaderboard", board));
    expect(q(p.root, ".entry")).toHaveLength(0);
    expect(p.root.querySelector(".empty")?.textContent).toBe("—");
  });

  it("SEGURIDAD: apodos con HTML y avatares peligrosos no crean elementos", () => {
    const p = load("leaderboard");
    p.send(config("leaderboard", { ...cfg, showAvatar: true }));
    p.send(overlayMsg("leaderboard", { session: [entry('<img src=x onerror="window.__p=1">', 5, "javascript:alert(1)")], day: [], all: [] }));
    expect(p.root.querySelectorAll("img")).toHaveLength(0);
    expect(p.root.textContent).toContain("<img src=x");
    expect((p.win as unknown as { __p?: number }).__p).toBeUndefined();
  });
});

describe("metas", () => {
  const cfg = { width: 400, trackColor: "#333333", barHeight: 20, goalId: "", showLabel: true, showNumbers: true, showPercent: true };
  const goal = (id: string, name: string, current: number, target: number) => ({ id, name, kind: "likes", current, target, percent: Math.min(100, (current / target) * 100), reachedCount: 0 });

  it("dibuja la barra con el porcentaje y las cifras", () => {
    const p = load("goals");
    p.send(config("goals", cfg));
    p.send(overlayMsg("goals", { goals: [goal("a", "Likes", 2500, 10000)] }));
    const fill = p.root.querySelector<HTMLElement>(".fill");
    expect(fill?.style.width).toBe("25%");
    expect(p.root.textContent).toContain("Likes");
    expect(p.root.textContent).toContain("2500 / 10.000");
    expect(p.root.textContent).toContain("25%");
  });

  it("reutiliza la barra al avanzar (para que se anime) y marca la meta completa", () => {
    const p = load("goals");
    p.send(config("goals", cfg));
    p.send(overlayMsg("goals", { goals: [goal("a", "Likes", 100, 1000)] }));
    const first = p.root.querySelector(".hb-card");
    p.send(overlayMsg("goals", { goals: [goal("a", "Likes", 1000, 1000)] }));
    expect(p.root.querySelector(".hb-card")).toBe(first);
    expect(p.root.querySelector<HTMLElement>(".fill")?.style.width).toBe("100%");
    expect(first?.classList.contains("done")).toBe(true);
  });

  it("muestra solo la meta elegida y retira las que desaparecen", () => {
    const p = load("goals");
    p.send(config("goals", { ...cfg, goalId: "b" }));
    p.send(overlayMsg("goals", { goals: [goal("a", "Una", 1, 10), goal("b", "Otra", 5, 10)] }));
    expect(q(p.root, ".hb-card")).toHaveLength(1);
    expect(p.root.textContent).toContain("Otra");
    p.send(config("goals", { ...cfg, goalId: "" }));
    expect(q(p.root, ".hb-card")).toHaveLength(2);
    p.send(overlayMsg("goals", { goals: [goal("b", "Otra", 5, 10)] }));
    expect(q(p.root, ".hb-card")).toHaveLength(1);
  });

  it("una meta con porcentaje fuera de rango no desborda la barra", () => {
    const p = load("goals");
    p.send(config("goals", cfg));
    p.send(overlayMsg("goals", { goals: [{ ...goal("a", "X", 0, 1), percent: 480 }, { ...goal("b", "Y", 0, 1), percent: -5 }] }));
    const widths = q<HTMLElement>(p.root, ".fill").map((f) => f.style.width);
    expect(widths).toEqual(["100%", "0%"]);
  });

  it("SEGURIDAD: el nombre de la meta se muestra como texto", () => {
    const p = load("goals");
    p.send(config("goals", cfg));
    p.send(overlayMsg("goals", { goals: [goal("a", "<b>negrita</b><img src=x>", 1, 10)] }));
    expect(p.root.querySelector("b")).toBeNull();
    expect(p.root.querySelector("img")).toBeNull();
  });
});

describe("timer", () => {
  const cfg = { digitSize: 64, lowColor: "#ef4444", timerId: "", showLabel: true, format: "auto", lowSeconds: 60, endText: "¡Tiempo!" };
  const digits = (root: HTMLElement) => root.querySelector(".digits")?.textContent;

  it("cuenta hacia atrás desde la hora de fin y marca el poco tiempo", async () => {
    const p = load("timer");
    p.send(config("timer", cfg));
    p.send(overlayMsg("timer", { timers: [{ id: "t", name: "Subathon", status: "running", remainingMs: 0, endsAtMs: Date.now() + 125_000 }] }));
    expect(digits(p.root)).toMatch(/^02:0[3-5]$/);
    expect(p.root.querySelector(".label")?.textContent).toBe("Subathon");
    expect(p.root.querySelector(".timer")?.classList.contains("low")).toBe(false);

    p.send(overlayMsg("timer", { timers: [{ id: "t", name: "Subathon", status: "running", remainingMs: 0, endsAtMs: Date.now() + 30_000 }] }));
    expect(p.root.querySelector(".timer")?.classList.contains("low")).toBe(true);
    const before = digits(p.root);
    await sleep(1_300);
    expect(digits(p.root)).not.toBe(before);
    p.win.close();
  });

  it("formato de horas y formato forzado minutos:segundos", () => {
    const p = load("timer");
    p.send(config("timer", cfg));
    p.send(overlayMsg("timer", { timers: [{ id: "t", name: "T", status: "paused", remainingMs: 3_725_000, endsAtMs: 0 }] }));
    expect(digits(p.root)).toBe("01:02:05");
    p.send(config("timer", { ...cfg, format: "ms" }));
    expect(digits(p.root)).toBe("62:05");
    expect(p.root.querySelector(".timer")?.classList.contains("paused")).toBe(true);
  });

  it("al terminar muestra el texto configurado", () => {
    const p = load("timer");
    p.send(config("timer", { ...cfg, endText: "FIN" }));
    p.send(overlayMsg("timer", { timers: [{ id: "t", name: "T", status: "ended", remainingMs: 0, endsAtMs: 0 }] }));
    expect(digits(p.root)).toBe("FIN");
  });

  it("elige el timer por ID y se oculta si no existe", () => {
    const p = load("timer");
    p.send(config("timer", { ...cfg, timerId: "b" }));
    p.send(overlayMsg("timer", { timers: [{ id: "a", name: "A", status: "paused", remainingMs: 1000, endsAtMs: 0 }, { id: "b", name: "B", status: "paused", remainingMs: 61_000, endsAtMs: 0 }] }));
    expect(digits(p.root)).toBe("01:01");
    p.send(config("timer", { ...cfg, timerId: "zzz" }));
    expect(p.root.querySelector<HTMLElement>(".timer")?.style.display).toBe("none");
    p.win.close();
  });
});

describe("contadores", () => {
  const cfg = { showLikes: true, showViewers: true, likesLabel: "Likes", viewersLabel: "Espectadores", layout: "row" };

  it("muestra likes y espectadores con separador de miles", () => {
    const p = load("counters");
    p.send(config("counters", cfg));
    p.send(overlayMsg("counters", { likes: 12345, viewers: 678, peakViewers: 900 }));
    const text = p.root.textContent ?? "";
    expect(text).toContain("12.345");
    expect(text).toContain("678");
    expect(text).toContain("Likes");
    expect(text).toContain("Espectadores");
  });

  it("puede ocultar cada contador, usar etiquetas propias y disposición en columna", () => {
    const p = load("counters");
    p.send(overlayMsg("counters", { likes: 5, viewers: 7, peakViewers: 7 }));
    p.send(config("counters", { ...cfg, showLikes: false, viewersLabel: "En vivo", layout: "column" }));
    expect(p.root.textContent).not.toContain("Likes");
    expect(p.root.textContent).toContain("En vivo");
    expect(p.root.querySelector(".counters")?.classList.contains("column")).toBe(true);
    p.send(config("counters", { ...cfg, showLikes: false, showViewers: false }));
    expect(p.root.querySelector<HTMLElement>(".counters")?.style.display).toBe("none");
  });

  it("SEGURIDAD: una etiqueta configurada se muestra como texto", () => {
    const p = load("counters");
    p.send(config("counters", { ...cfg, likesLabel: "<img src=x onerror=1>" }));
    p.send(overlayMsg("counters", { likes: 1, viewers: 1, peakViewers: 1 }));
    expect(p.root.querySelector("img")).toBeNull();
  });
});
