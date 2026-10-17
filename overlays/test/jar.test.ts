import { describe, expect, it } from "vitest";
import { config, eventMsg, gift, load, overlayMsg } from "./harness";

const cfg = {
  jarSize: 320,
  liquidColor: "#ffc113",
  glassColor: "#ffffff",
  goalId: "",
  showGoal: true,
  showUsd: true,
  usdPerCoin: 0.005,
  currencySymbol: "$",
  dropGifts: true,
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

const goals = (...list: unknown[]) => overlayMsg("goals", { goals: list });
const text = (p: ReturnType<typeof load>, sel: string) => p.root.querySelector(sel)?.textContent ?? "";
const liquid = (p: ReturnType<typeof load>) => p.root.querySelector<HTMLElement>(".liquid")?.style.transform ?? "";

describe("jarrón de regalos", () => {
  it("llena el jarrón según la meta de monedas y estima los dólares", () => {
    const p = load("jar");
    p.send(config("jar", cfg));
    p.send(goals(goal()));
    // 25 % lleno: queda un 75 % del interior (190) por subir.
    expect(liquid(p)).toBe("translateY(142.5px)");
    expect(text(p, ".usd")).toBe("≈ $5.00");
    expect(text(p, ".nums")).toMatch(/^1.?000 \/ 4.?000 monedas$/);
    expect(text(p, ".est")).toBe("estimado");
  });

  it("usa la tasa configurada y el símbolo elegido", () => {
    const p = load("jar");
    p.send(config("jar", { ...cfg, usdPerCoin: 0.01, currencySymbol: "US$" }));
    p.send(goals(goal({ current: 250 })));
    expect(text(p, ".usd")).toBe("≈ US$2.50");
  });

  it("ignora metas que no son de monedas, salvo que se pida una por id", () => {
    const p = load("jar");
    p.send(config("jar", cfg));
    p.send(goals(goal({ id: "likes", kind: "likes", current: 9, percent: 90 }), goal({ id: "coins", current: 2000, percent: 50 })));
    expect(text(p, ".nums")).toMatch(/^2.?000 \//);
    p.send(config("jar", { ...cfg, goalId: "likes" }));
    expect(text(p, ".nums")).toContain("9 /");
  });

  it("sin meta de monedas el jarrón queda vacío y cuenta los regalos vistos", () => {
    const p = load("jar");
    p.send(config("jar", cfg));
    p.send(eventMsg(gift("Rose", 100, 1)));
    p.send(eventMsg(gift("Lion", 400, 1)));
    expect(liquid(p)).toBe("translateY(190px)");
    expect(text(p, ".usd")).toBe("≈ $2.50");
  });

  it("al llegar al 100 % marca el jarrón como lleno y nunca se sale del cristal", () => {
    const p = load("jar");
    p.send(config("jar", cfg));
    p.send(goals(goal({ current: 9000, percent: 225 })));
    expect(liquid(p)).toBe("translateY(0px)");
    expect(p.root.querySelector(".jar-wrap")?.classList.contains("full")).toBe(true);
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

  it("rechaza imágenes que no sean HTTPS y se puede ocultar el dólar", () => {
    const p = load("jar");
    p.send(config("jar", { ...cfg, showUsd: false }));
    p.send(eventMsg(gift("Lion", 500, 1, "http://inseguro/lion.png")));
    expect(p.root.querySelectorAll(".drop")).toHaveLength(0);
    expect(p.root.querySelector<HTMLElement>(".usd")?.style.display).toBe("none");
  });
});
