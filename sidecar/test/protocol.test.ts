import { describe, expect, it } from "vitest";
import { encode, parseCommand } from "../src/protocol";

describe("protocol", () => {
  it("encode produce una sola línea NDJSON", () => {
    const line = encode({ kind: "status", state: "connected" });
    expect(line).toBe('{"kind":"status","state":"connected"}\n');
  });

  it("parseCommand acepta connect con y sin API key", () => {
    expect(parseCommand('{"cmd":"connect","uniqueId":"ana"}')).toEqual({
      cmd: "connect",
      uniqueId: "ana",
    });
    expect(parseCommand('{"cmd":"connect","uniqueId":"ana","eulerApiKey":"k"}')).toEqual({
      cmd: "connect",
      uniqueId: "ana",
      eulerApiKey: "k",
    });
  });

  it("parseCommand ignora una sesión de TikTok y rechaza el sendChat de versiones antiguas", () => {
    expect(parseCommand('{"cmd":"connect","uniqueId":"ana","session":{"sessionId":"s","ttTargetIdc":"t"}}')).toEqual({ cmd: "connect", uniqueId: "ana" });
    expect(parseCommand('{"cmd":"sendChat","requestId":"r1","text":"hola"}')).toBeNull();
  });

  it("parseCommand rechaza basura sin lanzar", () => {
    expect(parseCommand("no json")).toBeNull();
    expect(parseCommand("[]")).toBeNull();
    expect(parseCommand('{"cmd":"connect"}')).toBeNull();
    expect(parseCommand('{"cmd":"connect","uniqueId":"  "}')).toBeNull();
    expect(parseCommand('{"cmd":"otro"}')).toBeNull();
  });
});
