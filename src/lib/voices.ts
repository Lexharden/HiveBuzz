import type { VoiceInfo } from "./types";

const ENGINE_NAMES: Record<string, string> = { edge: "Edge", piper: "Piper", sapi: "Windows" };

/** Nombre legible de una voz: «es-MX-DaliaNeural» de Edge → «Dalia (Edge · es-MX)». */
export function voiceLabel(v: VoiceInfo): string {
  let name = v.name;
  if (v.engine === "edge") {
    name = name
      .replace(/^[a-z]{2,3}-[A-Z]{2}-/, "")
      .replace(/Neural$/, "")
      .replace(/Multilingual$/, " (multi)");
  }
  const engine = ENGINE_NAMES[v.engine] ?? v.engine;
  return `${name} (${engine}${v.lang ? ` · ${v.lang}` : ""})`;
}
