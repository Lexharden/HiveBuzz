// Arnés para probar los overlays tal como los sirve HiveBuzz: se arma la página igual que
// `src-tauri/src/server/pages.rs` (núcleo común inyectado) y se ejecuta en jsdom con un
// WebSocket falso que el test controla.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { JSDOM } from "jsdom";

const OVERLAYS_DIR = join(dirname(fileURLToPath(import.meta.url)), "..");
const MARKER = "<!--HB_COMMON-->";

/** HTML de un overlay con el núcleo común inyectado, igual que lo hace el servidor. */
export function pageHtml(id: string): string {
  const css = readFileSync(join(OVERLAYS_DIR, "common.css"), "utf8");
  const js = readFileSync(join(OVERLAYS_DIR, "common.js"), "utf8");
  const html = readFileSync(join(OVERLAYS_DIR, id, "index.html"), "utf8");
  return html.replace(MARKER, `<style>${css}</style><script>${js}</script>`);
}

export class FakeSocket {
  static instances: FakeSocket[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((m: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  constructor(public url: string) {
    FakeSocket.instances.push(this);
  }
  close(): void {
    this.closed = true;
    this.onclose?.();
  }
}

export interface Page {
  dom: JSDOM;
  win: Window & typeof globalThis;
  doc: Document;
  socket: FakeSocket;
  /** Entrega al overlay un mensaje del servidor. */
  send(msg: unknown): void;
  /** Entrega texto crudo (para probar mensajes malformados). */
  sendRaw(text: string): void;
  root: HTMLElement;
}

export interface LoadOptions {
  query?: string;
  /** Si se pasa, se instalan temporizadores falsos de vitest antes de cargar. */
  beforeLoad?: () => void;
}

export function load(id: string, opts: LoadOptions = {}): Page {
  FakeSocket.instances = [];
  opts.beforeLoad?.();
  const dom = new JSDOM(pageHtml(id), {
    url: `http://127.0.0.1:17890/overlay/${id}?token=tok${opts.query ?? ""}`,
    runScripts: "dangerously",
    pretendToBeVisual: true,
    beforeParse(window) {
      (window as unknown as { WebSocket: unknown }).WebSocket = FakeSocket;
    },
  });
  const socket = FakeSocket.instances[0];
  if (!socket) throw new Error(`el overlay ${id} no abrió un WebSocket`);
  socket.onopen?.();
  const win = dom.window as unknown as Window & typeof globalThis;
  const root = win.document.getElementById("hb-root");
  if (!root) throw new Error(`el overlay ${id} no tiene #hb-root`);
  return {
    dom,
    win,
    doc: win.document,
    socket,
    root,
    send: (msg) => socket.onmessage?.({ data: JSON.stringify(msg) }),
    sendRaw: (text) => socket.onmessage?.({ data: text }),
  };
}

// ---- Constructores de mensajes del servidor ----

export const config = (id: string, data: Record<string, unknown>) => ({ type: "overlay", channel: `config:${id}`, data });
export const overlayMsg = (channel: string, data: unknown) => ({ type: "overlay", channel, data });
export const eventMsg = (event: unknown) => ({ type: "event", event });
export const historyMsg = (events: unknown[]) => ({ type: "history", events });

interface UserOverrides {
  id?: string;
  uniqueId?: string;
  nickname?: string;
  avatar?: string;
  isModerator?: boolean;
  isSubscriber?: boolean;
}

export function user(o: UserOverrides = {}) {
  return {
    id: "1",
    uniqueId: "ana",
    nickname: "Ana",
    avatar: "",
    isModerator: false,
    isSubscriber: false,
    isFollower: false,
    ...o,
  };
}

let seq = 0;
export function chat(text: string, u: UserOverrides = {}, extra: Record<string, unknown> = {}) {
  return { id: `c${++seq}`, type: "chat", user: user(u), chat: { text, ...extra }, ts: 1 };
}

export function gift(name: string, coins: number, count = 1, image = "", u: UserOverrides = {}) {
  return { id: `g${++seq}`, type: "gift", user: user(u), gift: { id: 1, name, coins, count, streakable: false, image }, ts: 1 };
}

export function simple(type: string, u: UserOverrides = {}) {
  return { id: `e${++seq}`, type, user: user(u), ts: 1 };
}
