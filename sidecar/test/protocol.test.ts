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

  it("parseCommand acepta la sesión de TikTok y descarta una a medias", () => {
    expect(parseCommand('{"cmd":"connect","uniqueId":"ana","session":{"sessionId":"s","ttTargetIdc":"t"}}')).toEqual({
      cmd: "connect",
      uniqueId: "ana",
      session: { sessionId: "s", ttTargetIdc: "t" },
    });
    for (const half of ['{"sessionId":"s"}', '{"ttTargetIdc":"t"}', '{"sessionId":"","ttTargetIdc":"t"}', '"texto"', "null", "[]"]) {
      expect(parseCommand(`{"cmd":"connect","uniqueId":"ana","session":${half}}`), half).toEqual({ cmd: "connect", uniqueId: "ana" });
    }
  });

  it("parseCommand acepta sendChat y rechaza los incompletos", () => {
    expect(parseCommand('{"cmd":"sendChat","requestId":"r1","text":"hola"}')).toEqual({ cmd: "sendChat", requestId: "r1", text: "hola" });
    expect(parseCommand('{"cmd":"sendChat","requestId":"r1","text":""}')).toEqual({ cmd: "sendChat", requestId: "r1", text: "" });
    for (const bad of ['{"cmd":"sendChat"}', '{"cmd":"sendChat","requestId":"","text":"x"}', '{"cmd":"sendChat","requestId":"r","text":5}', '{"cmd":"sendChat","text":"x"}']) {
      expect(parseCommand(bad), bad).toBeNull();
    }
  });

  it("parseCommand rechaza basura sin lanzar", () => {
    expect(parseCommand("no json")).toBeNull();
    expect(parseCommand("[]")).toBeNull();
    expect(parseCommand('{"cmd":"connect"}')).toBeNull();
    expect(parseCommand('{"cmd":"connect","uniqueId":"  "}')).toBeNull();
    expect(parseCommand('{"cmd":"otro"}')).toBeNull();
  });
});
