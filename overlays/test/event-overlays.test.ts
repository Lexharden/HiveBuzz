import { describe, expect, it } from "vitest";
import { chat, config, eventMsg, gift, load, simple } from "./harness";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const cards = (root: HTMLElement) => [...root.querySelectorAll<HTMLElement>(".hb-card")];

describe("chat en pantalla", () => {
  it("muestra nombre y texto, insignias y respeta el máximo de mensajes", () => {
    const p = load("chat");
    p.send(config("chat", { maxMessages: 2, showBadges: true, showAvatar: false, hideCommands: true, showEmotes: true, hideUsers: "", lifetimeSec: 0, width: 400, nameColor: "accent" }));
    p.send(eventMsg(chat("uno", { nickname: "Ana", isModerator: true })));
    p.send(eventMsg(chat("dos", { nickname: "Beto", isSubscriber: true })));
    p.send(eventMsg(chat("tres", { nickname: "Cata" })));
    const list = cards(p.root);
    expect(list).toHaveLength(2);
    expect(list[0]?.textContent).toContain("Beto");
    expect(list[0]?.textContent).toContain("⭐");
    expect(list[1]?.textContent).toContain("Cata: tres");
    expect(p.root.style.width).toBe("400px");
  });

  it("oculta comandos y usuarios configurados", () => {
    const p = load("chat");
    p.send(config("chat", { maxMessages: 10, hideCommands: true, hideUsers: "@Spam, bot_x", showAvatar: false, showBadges: false, showEmotes: true, lifetimeSec: 0, width: 400, nameColor: "accent" }));
    p.send(eventMsg(chat("!puntos")));
    p.send(eventMsg(chat("hola", { uniqueId: "spam" })));
    p.send(eventMsg(chat("hola", { uniqueId: "Bot_X" })));
    p.send(eventMsg(chat("visible", { uniqueId: "otro" })));
    expect(cards(p.root)).toHaveLength(1);
    expect(p.root.textContent).toContain("visible");
  });

  it("ignora los eventos que no son de chat", () => {
    const p = load("chat");
    p.send(eventMsg(simple("follow")));
    p.send(eventMsg(gift("Rose", 1)));
    expect(cards(p.root)).toHaveLength(0);
  });

  it("SEGURIDAD: el texto y el apodo de TikTok jamás se interpretan como HTML", () => {
    const p = load("chat");
    p.send(config("chat", { maxMessages: 10, hideCommands: false, showAvatar: true, showBadges: true, showEmotes: true, lifetimeSec: 0, width: 400, nameColor: "accent", hideUsers: "" }));
    const evil = '<img src=x onerror="window.__pwned=1"><script>window.__pwned=2</script><b>negrita</b>';
    p.send(eventMsg(chat(evil, { nickname: evil })));
    expect(p.root.querySelector("img")).toBeNull();
    expect(p.root.querySelector("script")).toBeNull();
    expect(p.root.querySelector("b")).toBeNull();
    expect(p.root.textContent).toContain("<img src=x");
    expect((p.win as unknown as { __pwned?: number }).__pwned).toBeUndefined();
  });

  it("SEGURIDAD: un avatar o emote con URL peligrosa no crea imágenes", () => {
    const p = load("chat");
    p.send(config("chat", { maxMessages: 10, hideCommands: false, showAvatar: true, showBadges: false, showEmotes: true, lifetimeSec: 0, width: 400, nameColor: "accent", hideUsers: "" }));
    p.send(
      eventMsg(
        chat("hola", { avatar: "javascript:alert(1)" }, { emotes: [{ id: "1", image: "http://inseguro/e.png" }, { id: "2", image: "data:image/svg+xml,<svg/>" }] }),
      ),
    );
    expect(p.root.querySelectorAll("img")).toHaveLength(0);
    p.send(eventMsg(chat("ok", { avatar: "https://cdn.example/a.png" }, { emotes: [{ id: "3", image: "https://cdn.example/e.png" }] })));
    expect(p.root.querySelectorAll("img")).toHaveLength(2);
  });

  it("el color por usuario es estable", () => {
    const p = load("chat");
    p.send(config("chat", { maxMessages: 10, hideCommands: false, showAvatar: false, showBadges: false, showEmotes: true, lifetimeSec: 0, width: 400, nameColor: "perUser", hideUsers: "" }));
    p.send(eventMsg(chat("a", { id: "42" })));
    p.send(eventMsg(chat("b", { id: "42" })));
    p.send(eventMsg(chat("c", { id: "43" })));
    const colors = [...p.root.querySelectorAll<HTMLElement>(".hb-who")].map((e) => e.style.color);
    expect(colors[0]).toBe(colors[1]);
    expect(colors[0]).not.toBe(colors[2]);
  });

  it("los mensajes desaparecen pasado su tiempo de vida", async () => {
    const p = load("chat");
    p.send(config("chat", { maxMessages: 10, hideCommands: false, showAvatar: false, showBadges: false, showEmotes: true, lifetimeSec: 0.05, width: 400, nameColor: "accent", hideUsers: "" }));
    p.send(eventMsg(chat("efímero")));
    expect(cards(p.root)).toHaveLength(1);
    await sleep(900);
    expect(cards(p.root)).toHaveLength(0);
  });
});

describe("feed de eventos", () => {
  const cfg = { maxItems: 10, lifetimeSec: 0, showAvatar: false, showChat: true, showGift: true, showFollow: true, showShare: true, showSubscribe: true, showEmote: false };

  it("describe cada tipo de evento en español", () => {
    const p = load("feed");
    p.send(config("feed", cfg));
    p.send(eventMsg(simple("follow")));
    p.send(eventMsg(simple("share")));
    p.send(eventMsg(simple("subscribe")));
    p.send(eventMsg(gift("Rose", 30, 3)));
    p.send(eventMsg(chat("hola")));
    const texts = cards(p.root).map((c) => c.textContent);
    expect(texts[0]).toContain("empezó a seguirte");
    expect(texts[1]).toContain("compartió el LIVE");
    expect(texts[2]).toContain("se suscribió");
    expect(texts[3]).toContain("envió 3× Rose");
    expect(texts[3]).toMatch(/30\s+monedas/);
    expect(texts[4]).toContain("Ana: hola");
  });

  it("respeta los filtros por tipo y descarta likes y entradas", () => {
    const p = load("feed");
    p.send(config("feed", { ...cfg, showChat: false, showFollow: false }));
    p.send(eventMsg(chat("oculto")));
    p.send(eventMsg(simple("follow")));
    p.send(eventMsg(simple("like")));
    p.send(eventMsg(simple("join")));
    p.send(eventMsg(simple("share")));
    expect(cards(p.root)).toHaveLength(1);
    expect(p.root.textContent).toContain("compartió");
  });

  it("limita la cantidad de elementos", () => {
    const p = load("feed");
    p.send(config("feed", { ...cfg, maxItems: 3 }));
    for (let i = 0; i < 8; i++) p.send(eventMsg(chat(`m${i}`)));
    const list = cards(p.root);
    expect(list).toHaveLength(3);
    expect(list[2]?.textContent).toContain("m7");
  });
});

describe("regalos recientes", () => {
  const cfg = { width: 340, maxItems: 5, lifetimeSec: 0, minCoins: 10, showImage: true, showAvatar: false, showCoins: true };

  it("solo muestra regalos desde las monedas mínimas, con su imagen HTTPS", () => {
    const p = load("gifts");
    p.send(config("gifts", cfg));
    p.send(eventMsg(gift("Rose", 1, 1, "https://cdn.example/rose.png")));
    p.send(eventMsg(gift("Lion", 29999, 1, "https://cdn.example/lion.png")));
    p.send(eventMsg(chat("no es regalo")));
    const list = cards(p.root);
    expect(list).toHaveLength(1);
    expect(list[0]?.textContent).toContain("Lion");
    expect(list[0]?.textContent).toMatch(/29\.999\s+monedas/);
    expect(list[0]?.querySelector("img")?.getAttribute("src")).toBe("https://cdn.example/lion.png");
  });

  it("SEGURIDAD: una imagen de regalo que no es HTTPS se ignora", () => {
    const p = load("gifts");
    p.send(config("gifts", { ...cfg, minCoins: 0 }));
    p.send(eventMsg(gift("A", 5, 1, "javascript:alert(1)")));
    p.send(eventMsg(gift("B", 5, 1, "http://inseguro/x.png")));
    expect(cards(p.root)).toHaveLength(2);
    expect(p.root.querySelectorAll("img")).toHaveLength(0);
  });

  it("puede ocultar monedas e imagen", () => {
    const p = load("gifts");
    p.send(config("gifts", { ...cfg, minCoins: 0, showCoins: false, showImage: false }));
    p.send(eventMsg(gift("Rose", 5, 1, "https://cdn.example/rose.png")));
    expect(p.root.textContent).not.toContain("monedas");
    expect(p.root.querySelector("img")).toBeNull();
  });
});

describe("plataforma de origen", () => {
  const chatCfg = { maxMessages: 10, hideCommands: false, showAvatar: false, showBadges: false, showEmotes: true, lifetimeSec: 0, width: 400, nameColor: "accent", hideUsers: "" };

  it("la insignia solo aparece si se activa, y distingue TikTok de Twitch", () => {
    const off = load("chat");
    off.send(config("chat", { ...chatCfg, showPlatform: false }));
    off.send(eventMsg({ ...chat("hola"), platform: "twitch" }));
    expect(off.root.querySelector(".hb-platform")).toBeNull();

    const on = load("chat");
    on.send(config("chat", { ...chatCfg, showPlatform: true }));
    on.send(eventMsg({ ...chat("a"), platform: "twitch" }));
    on.send(eventMsg({ ...chat("b"), platform: "tiktok" }));
    on.send(eventMsg(chat("c"))); // eventos antiguos sin plataforma = TikTok
    const badges = [...on.root.querySelectorAll<HTMLElement>(".hb-platform")];
    expect(badges.map((b) => b.textContent)).toEqual(["Twitch", "TikTok", "TikTok"]);
    expect(badges[0]?.classList.contains("twitch")).toBe(true);
  });

  it("SEGURIDAD: una plataforma desconocida o con HTML no se inserta ni como clase", () => {
    const p = load("chat");
    p.send(config("chat", { ...chatCfg, showPlatform: true }));
    p.send(eventMsg({ ...chat("x"), platform: '"><img src=x onerror=alert(1)>' }));
    const b = p.root.querySelector<HTMLElement>(".hb-platform");
    expect(b?.textContent).toBe("TikTok");
    expect(b?.className).toBe("hb-platform tiktok");
    expect(p.root.querySelector("img")).toBeNull();
  });

  it("los regalos de Twitch se muestran en bits y los de TikTok en monedas", () => {
    const p = load("gifts");
    p.send(config("gifts", { width: 340, maxItems: 6, lifetimeSec: 0, minCoins: 0, showImage: false, showAvatar: false, showCoins: true, showPlatform: true }));
    p.send(eventMsg({ ...gift("Bits", 100), platform: "twitch" }));
    p.send(eventMsg(gift("Rose", 5)));
    const texts = cards(p.root).map((c) => c.textContent ?? "");
    expect(texts[0]).toContain("100 bits");
    expect(texts[0]).toContain("Twitch");
    expect(texts[1]).toContain("5 monedas");
  });

  it("el feed también usa bits para Twitch", () => {
    const p = load("feed");
    p.send(config("feed", { maxItems: 12, lifetimeSec: 0, showAvatar: false, showPlatform: false }));
    p.send(eventMsg({ ...gift("Bits", 300), platform: "twitch" }));
    expect(p.root.textContent).toContain("300 bits");
  });
});
