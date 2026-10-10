import { describe, expect, it } from "vitest";
import en from "../../src/i18n/en.json";
import es from "../../src/i18n/es.json";
import { isActive, isHealthy, statusKey, TWITCH_DETAIL } from "../../src/lib/connectionText";
import { countByPlatform, filterEvents, platformOf } from "../../src/lib/feedFilter";
import { computeSteps, nextStep, progress } from "../../src/lib/gettingStarted";
import { ALL_PAGES, DOCUMENTED, ICONS, NAV } from "../../src/lib/nav";
import { isSatisfied, placePopover, TOUR } from "../../src/lib/tour";
import type { LiveEvent, StatusUpdate } from "../../src/lib/types";

type Tree = { [k: string]: string | Tree };
const lookup = (tree: Tree, key: string): string | undefined => {
  let cur: string | Tree | undefined = tree;
  for (const part of key.split(".")) {
    if (typeof cur !== "object" || cur === undefined) return undefined;
    cur = cur[part];
  }
  return typeof cur === "string" ? cur : undefined;
};
const both = (key: string) => {
  expect(lookup(es as Tree, key), `es: ${key}`).toBeTruthy();
  expect(lookup(en as Tree, key), `en: ${key}`).toBeTruthy();
};

const status = (state: StatusUpdate["state"], detail: string | null = null): StatusUpdate => ({ state, detail, attempt: null, retryInMs: null });
const ev = (id: string, platform?: "tiktok" | "twitch"): LiveEvent => ({
  id,
  type: "chat",
  platform,
  user: { id: "1", uniqueId: "a", nickname: "A", avatar: "", isModerator: false, isSubscriber: false, isFollower: false },
  ts: 1,
});

describe("texto de los estados de conexión", () => {
  const states: StatusUpdate["state"][] = ["disconnected", "waiting_live", "connected", "signature_error", "reconnecting"];
  const details = [null, TWITCH_DETAIL.chat, TWITCH_DETAIL.full, TWITCH_DETAIL.notOwner, TWITCH_DETAIL.connecting, "algo-raro"];

  it("toda combinación de plataforma, estado y detalle tiene frase en español e inglés", () => {
    for (const p of ["tiktok", "twitch"] as const) for (const s of states) for (const d of details) both(statusKey(p, status(s, d)));
    for (const s of states) both(`conn.short.${s}`);
  });

  it("Twitch distingue lo que está recibiendo", () => {
    expect(statusKey("twitch", status("connected", TWITCH_DETAIL.full))).toBe("conn.state.twitch.full");
    expect(statusKey("twitch", status("connected", TWITCH_DETAIL.notOwner))).toBe("conn.state.twitch.notOwner");
    expect(statusKey("twitch", status("connected", null))).toBe("conn.state.twitch.chat");
    expect(statusKey("twitch", status("waiting_live", TWITCH_DETAIL.connecting))).toBe("conn.state.twitch.connecting");
    expect(statusKey("twitch", status("waiting_live", TWITCH_DETAIL.full))).toBe("conn.state.twitch.offline");
    expect(statusKey("tiktok", status("waiting_live"))).toBe("conn.state.tiktok.waitingLive");
  });

  it("«activo» y «sano» no son lo mismo", () => {
    expect(isActive(status("disconnected"))).toBe(false);
    expect(isActive(status("reconnecting"))).toBe(true);
    expect(isHealthy(status("reconnecting"))).toBe(false);
    expect(isHealthy(status("connected"))).toBe(true);
  });
});

describe("filtro del feed", () => {
  const events = [ev("1"), ev("2", "twitch"), ev("3", "tiktok"), ev("4", "twitch")];
  it("sin plataforma es TikTok (eventos antiguos y del sidecar)", () => {
    expect(platformOf(ev("x"))).toBe("tiktok");
  });
  it("filtra y cuenta por plataforma", () => {
    expect(filterEvents(events, "all")).toHaveLength(4);
    expect(filterEvents(events, "twitch").map((e) => e.id)).toEqual(["2", "4"]);
    expect(filterEvents(events, "tiktok").map((e) => e.id)).toEqual(["1", "3"]);
    expect(countByPlatform(events)).toEqual({ tiktok: 2, twitch: 2 });
    expect(countByPlatform([])).toEqual({ tiktok: 0, twitch: 0 });
  });
});

describe("primeros pasos", () => {
  it("marca cada paso según lo que ya se hizo y propone el siguiente", () => {
    let steps = computeSteps({ connectedEver: false, overlayDone: false, testedDone: false, rules: 0 });
    expect(steps.map((s) => s.id)).toEqual(["connect", "overlay", "test", "rule"]);
    expect(progress(steps)).toEqual({ done: 0, total: 4, complete: false });
    expect(nextStep(steps)?.id).toBe("connect");
    steps = computeSteps({ connectedEver: true, overlayDone: false, testedDone: true, rules: 0 });
    expect(progress(steps).done).toBe(2);
    expect(nextStep(steps)?.id).toBe("overlay");
    steps = computeSteps({ connectedEver: true, overlayDone: true, testedDone: true, rules: 3 });
    expect(progress(steps).complete).toBe(true);
    expect(nextStep(steps)).toBeUndefined();
  });

  it("cada paso tiene título y pista en ambos idiomas", () => {
    for (const id of ["connect", "overlay", "test", "rule"]) {
      both(`start.${id}.title`);
      both(`start.${id}.hint`);
    }
  });
});

describe("menú", () => {
  it("las doce pantallas aparecen una sola vez, con icono, nombre y explicación en ambos idiomas", () => {
    expect(ALL_PAGES).toHaveLength(12);
    expect(new Set(ALL_PAGES).size).toBe(12);
    for (const p of ALL_PAGES) {
      expect(ICONS[p]).toBeTruthy();
      both(`tabs.${p}`);
      both(`pageHints.${p}`);
    }
    for (const g of NAV) if (g.key) both(`nav.groups.${g.key}`);
  });

  it("Inicio es lo primero del menú", () => {
    expect(NAV[0]?.pages[0]).toBe("dashboard");
  });
});

describe("ayuda", () => {
  it("cada pantalla documentada tiene explicación y cuatro puntos en ambos idiomas", () => {
    expect(DOCUMENTED).toHaveLength(10);
    expect(DOCUMENTED).not.toContain("help");
    expect(DOCUMENTED).not.toContain("about");
    for (const p of DOCUMENTED) {
      both(`help.sections.${p}.what`);
      for (const n of [1, 2, 3, 4]) both(`help.sections.${p}.i${n}`);
    }
  });

  it("glosario y preguntas frecuentes completos en ambos idiomas", () => {
    for (const g of ["event", "rule", "trigger", "action", "overlay", "session", "points", "tts", "cooldown"]) {
      both(`help.glossary.${g}.term`);
      both(`help.glossary.${g}.def`);
    }
    for (const f of ["obs", "bot", "spotify", "safe", "test", "update"]) {
      both(`help.faq.${f}.q`);
      both(`help.faq.${f}.a`);
    }
  });
});

describe("recorrido interactivo", () => {
  it("todos los pasos tienen sus textos en ambos idiomas", () => {
    for (const step of TOUR) {
      if (step.explains) {
        both(`tabs.${step.explains}`);
        both(`help.sections.${step.explains}.what`);
      } else {
        if (step.id !== "help") both(`tour.steps.${step.id}.title`);
        both(`tour.steps.${step.id}.body`);
      }
      if (step.wait) both(step.wait.kind === "flag" ? `tour.waiting.${step.wait.key.replace(".", "_")}` : "tour.waiting.page");
    }
    for (const k of ["counter", "next", "back", "skip", "finish", "done", "offer.title", "offer.body", "offer.start", "offer.later"]) both(`tour.${k}`);
  });

  it("los pasos son únicos y los que apuntan al menú existen en él", () => {
    expect(new Set(TOUR.map((s) => s.id)).size).toBe(TOUR.length);
    for (const s of TOUR) {
      if (s.target?.startsWith("nav-")) expect(ALL_PAGES).toContain(s.target.slice(4));
      if (s.page) expect(ALL_PAGES).toContain(s.page);
      if (s.wait?.kind === "page") expect(ALL_PAGES).toContain(s.wait.page);
    }
    expect(TOUR[0]?.id).toBe("welcome");
    expect(TOUR.at(-1)?.id).toBe("finish");
  });

  it("recorre todas las partes del menú (menos Inicio, que ya se vio, y «Acerca de»)", () => {
    const visited = new Set(TOUR.flatMap((s) => (s.explains ? [s.explains] : s.id === "help" ? ["help"] : [])));
    for (const p of ALL_PAGES.filter((x) => x !== "dashboard" && x !== "about")) expect(visited, p).toContain(p);
  });

  it("un paso con condición espera a que se cumpla; sin condición siempre está listo", () => {
    const sim = TOUR.find((s) => s.id === "simulate")!;
    const go = TOUR.find((s) => s.id === "goRules")!;
    expect(isSatisfied(sim, { page: "dashboard", flag: () => false })).toBe(false);
    expect(isSatisfied(sim, { page: "dashboard", flag: (k) => k === "gs.tested" })).toBe(true);
    expect(isSatisfied(go, { page: "dashboard", flag: () => false })).toBe(false);
    expect(isSatisfied(go, { page: "rules", flag: () => false })).toBe(true);
    expect(isSatisfied(TOUR[0]!, { page: "settings", flag: () => false })).toBe(true);
  });
});

describe("posición del cuadro del tutorial", () => {
  const view = { w: 1000, h: 700 };
  const pop = { w: 350, h: 200 };
  const inside = (p: { left: number; top: number }) => p.left >= 0 && p.top >= 0 && p.left + pop.w <= view.w && p.top + pop.h <= view.h;

  it("sin elemento va centrado", () => {
    expect(placePopover(null, pop, view)).toEqual({ left: 325, top: 250 });
  });

  it("junto al menú (izquierda) va a la derecha del elemento", () => {
    const p = placePopover({ left: 10, top: 100, width: 180, height: 36 }, pop, view);
    expect(p.left).toBe(10 + 180 + 14);
    expect(inside(p)).toBe(true);
  });

  it("si no cabe a la derecha, va debajo; si tampoco, encima; si nada cabe, centrado", () => {
    const right = placePopover({ left: 800, top: 100, width: 150, height: 40 }, pop, view);
    expect(right.top).toBe(100 + 40 + 14);
    expect(inside(right)).toBe(true);
    const above = placePopover({ left: 800, top: 600, width: 150, height: 60 }, pop, view);
    expect(above.top).toBe(600 - 14 - 200);
    const huge = placePopover({ left: 0, top: 0, width: 1000, height: 700 }, pop, view);
    expect(huge).toEqual({ left: 325, top: 250 });
  });

  it("nunca se sale de la ventana, ni con ventanas diminutas", () => {
    for (const t of [{ left: -50, top: -50, width: 80, height: 30 }, { left: 990, top: 690, width: 40, height: 40 }, { left: 500, top: 350, width: 10, height: 10 }]) {
      const p = placePopover(t, pop, view);
      expect(inside(p), JSON.stringify(t)).toBe(true);
    }
    const tiny = placePopover({ left: 5, top: 5, width: 20, height: 20 }, pop, { w: 300, h: 150 });
    expect(Number.isFinite(tiny.left) && Number.isFinite(tiny.top)).toBe(true);
  });
});
