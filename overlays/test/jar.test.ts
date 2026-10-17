import { describe, expect, it } from "vitest";
import { config, eventMsg, gift, load, overlayMsg } from "./harness";

const cfg = {
  shape: "jar",
  jarSize: 320,
  liquidColor: "#ffc113",
  liquidColor2: "#ff8a00",
  glassColor: "#ffffff",
  glassOpacity: 100,
  showLid: true,
  lidColor: "#ffffff",
  waves: true,
  bubbles: true,
  fillSpeed: 0.9,
  title: "",
  labelPosition: "below",
  showPercent: false,
  source: "own",
  target: 4000,
  onFull: "keep",
  fullEffect: "pulse",
  goalId: "",
  showGoal: true,
  showUsd: true,
  usdPerCoin: 0.005,
  currencySymbol: "$",
  dropGifts: true,
  dropMode: "gift",
  dropSize: 15,
  minCoins: 1,
};

const goal = (over: Record<string, unknown> = {}) => ({
  id: "g1",
  name: "Meta",
  kind: "coins",
  current: 1000,
  target: 4000,
  percent: 25,
  reachedCount: 0,
  ...over,
});

const jar = (coins: number, laps = 0) => overlayMsg("jar", { coins, laps });
const goals = (...list: unknown[]) => overlayMsg("goals", { goals: list });
const text = (p: ReturnType<typeof load>, sel: string) => p.root.querySelector(sel)?.textContent ?? "";
const liquid = (p: ReturnType<typeof load>) => p.root.querySelector<HTMLElement>(".liquid")?.style.transform ?? "";

describe("jarrón de regalos", () => {
  it("se llena con su propia cuenta, sin necesitar ninguna meta, y estima los dólares", () => {
    const p = load("jar");
    p.send(config("jar", cfg));
    p.send(jar(1000));
    // 25 % lleno: la superficie baja desde 56 por el 75 % del interior (190).
    expect(liquid(p)).toBe("translateY(198.5px)");
    expect(text(p, ".usd")).toBe("≈ $5.00");
    expect(text(p, ".nums")).toMatch(/^1.?000 \/ 4.?000 monedas$/);
    expect(text(p, ".est")).toBe("estimado");
  });

  it("al vaciarlo vuelve a cero", () => {
    const p = load("jar");
    p.send(config("jar", cfg));
    p.send(jar(2000));
    p.send(jar(0));
    expect(text(p, ".nums")).toMatch(/^0 \/ 4.?000 monedas$/);
    expect(liquid(p)).toBe("translateY(253px)");
  });

  it("con «vaciar al llenarse» los dólares suman las vueltas", () => {
    const p = load("jar");
    p.send(config("jar", { ...cfg, onFull: "reset", target: 1000 }));
    p.send(jar(500, 2));
    // 2 vueltas × 1000 + 500 monedas × 0,005.
    expect(text(p, ".usd")).toBe("≈ $12.50");
  });

  it("cada forma tiene su propio rango de llenado", () => {
    const p = load("jar");
    p.send(config("jar", { ...cfg, shape: "heart" }));
    p.send(jar(4000));
    expect(liquid(p)).toBe("translateY(62px)");
  });

  it("aplica colores, título, porcentaje y posición de los textos", () => {
    const p = load("jar");
    p.send(config("jar", { ...cfg, title: "Meta del día", showPercent: true, labelPosition: "above", liquidColor: "#112233", liquidColor2: "#445566" }));
    p.send(jar(1000));
    expect(text(p, ".title")).toBe("Meta del día");
    expect(text(p, ".pct")).toBe("25%");
    expect(p.root.querySelector<HTMLElement>(".totals")?.style.order).toBe("-1");
    expect(p.root.querySelector("stop[offset='1']")?.getAttribute("stop-color")).toBe("#112233");
    expect(p.root.querySelector("stop[offset='0']")?.getAttribute("stop-color")).toBe("#445566");
  });

  it("las olas y las burbujas se pueden quitar", () => {
    const p = load("jar");
    p.send(config("jar", cfg));
    expect(p.root.querySelectorAll(".bubble").length).toBeGreaterThan(0);
    p.send(config("jar", { ...cfg, bubbles: false, waves: false, showLid: false }));
    expect(p.root.querySelectorAll(".bubble")).toHaveLength(0);
    expect(p.root.querySelector(".wave")?.classList.contains("still")).toBe(true);
  });

  it("con origen «meta» sigue la meta de monedas, o la elegida por id", () => {
    const p = load("jar");
    p.send(config("jar", { ...cfg, source: "goal" }));
    p.send(goals(goal({ id: "likes", kind: "likes", current: 9, percent: 90 }), goal({ id: "coins", current: 2000, percent: 50 })));
    expect(text(p, ".nums")).toMatch(/^2.?000 \//);
    p.send(config("jar", { ...cfg, source: "goal", goalId: "likes" }));
    expect(text(p, ".nums")).toContain("9 /");
  });

  it("al llegar al 100 % marca el jarrón como lleno y nunca se sale del cristal", () => {
    const p = load("jar");
    p.send(config("jar", cfg));
    p.send(jar(9000));
    expect(liquid(p)).toBe("translateY(56px)");
    expect(p.root.querySelector(".jar-wrap")?.classList.contains("full")).toBe(true);
  });

  it("celebra al llenarse con confeti, pero no por el estado inicial", () => {
    const p = load("jar");
    p.send(config("jar", { ...cfg, fullEffect: "confetti" }));
    p.send(jar(5000));
    expect(p.root.querySelectorAll(".confetti")).toHaveLength(0);
    p.send(jar(0));
    p.send(jar(4000));
    expect(p.root.querySelectorAll(".confetti").length).toBeGreaterThan(0);
  });

  it("deja caer la imagen del regalo, pero no en el historial ni por debajo del mínimo", () => {
    const p = load("jar");
    p.send(config("jar", { ...cfg, minCoins: 50 }));
    p.send(eventMsg(gift("Rose", 1, 1, "https://cdn.example/rose.png")));
    expect(p.root.querySelectorAll(".drop")).toHaveLength(0);
    p.send(eventMsg(gift("Lion", 500, 1, "https://cdn.example/lion.png")));
    expect(p.root.querySelectorAll(".drop")).toHaveLength(1);
    p.send({ type: "history", events: [gift("Old", 500, 1, "https://cdn.example/old.png")] });
    expect(p.root.querySelectorAll(".drop")).toHaveLength(1);
  });

  it("puede dejar caer una moneda en vez de la imagen", () => {
    const p = load("jar");
    p.send(config("jar", { ...cfg, dropMode: "coin" }));
    p.send(eventMsg(gift("Lion", 500, 1, "")));
    expect(p.root.querySelector(".drop")?.textContent).toBe("🪙");
  });

  it("rechaza imágenes que no sean HTTPS y se puede ocultar el dólar", () => {
    const p = load("jar");
    p.send(config("jar", { ...cfg, showUsd: false }));
    p.send(eventMsg(gift("Lion", 500, 1, "http://inseguro/lion.png")));
    expect(p.root.querySelectorAll(".drop")).toHaveLength(0);
    expect(p.root.querySelector<HTMLElement>(".usd")?.style.display).toBe("none");
  });
});
