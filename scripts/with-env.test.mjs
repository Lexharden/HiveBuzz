import { describe, expect, it } from "vitest";
import { check, parseEnv } from "./with-env.mjs";

describe("parseEnv", () => {
  it("lee claves, ignora comentarios y líneas vacías, quita comillas y comentarios al final", () => {
    const env = parseEnv(`# comentario\n\nA=1\nB = "dos palabras"\nC='x'\nD=valor # nota\nE=\n  F=ok  \nmala línea\n1X=no\n`);
    expect(env).toEqual({ A: "1", B: "dos palabras", C: "x", D: "valor", E: "", F: "ok" });
  });

  it("acepta fin de línea de Windows y valores con =", () => {
    expect(parseEnv("A=b=c\r\nD=e\r\n")).toEqual({ A: "b=c", D: "e" });
  });
});

describe("check", () => {
  const id = "a".repeat(32);
  it("un Client ID vacío es solo un aviso", () => {
    const r = check({}, "tauri dev");
    expect(r.problems).toEqual([]);
    expect(r.notes.join(" ")).toContain("HIVEBUZZ_SPOTIFY_CLIENT_ID");
  });

  it("un Client ID con basura es un problema", () => {
    expect(check({ HIVEBUZZ_SPOTIFY_CLIENT_ID: "tiene espacios!" }, "tauri dev").problems).toHaveLength(1);
    expect(check({ HIVEBUZZ_SPOTIFY_CLIENT_ID: id }, "tauri dev").problems).toEqual([]);
  });

  it("una ruta de clave inexistente es un problema", () => {
    expect(check({ HIVEBUZZ_SPOTIFY_CLIENT_ID: id, TAURI_SIGNING_PRIVATE_KEY_PATH: "Z:/no/existe.key" }, "tauri build").problems).toHaveLength(1);
  });

  it("al compilar sin clave de firma avisa, pero no bloquea", () => {
    const r = check({ HIVEBUZZ_SPOTIFY_CLIENT_ID: id }, "tauri build");
    expect(r.problems).toEqual([]);
    expect(r.notes.join(" ")).toContain("auto-actualización");
    expect(check({ HIVEBUZZ_SPOTIFY_CLIENT_ID: id, HIVEBUZZ_TWITCH_CLIENT_ID: id }, "tauri dev").notes).toEqual([]);
    expect(check({ HIVEBUZZ_SPOTIFY_CLIENT_ID: id }, "tauri dev").notes.join(" ")).toContain("TWITCH");
    expect(check({ HIVEBUZZ_SPOTIFY_CLIENT_ID: id, HIVEBUZZ_TWITCH_CLIENT_ID: "mal id" }, "tauri dev").problems).toHaveLength(1);
  });
});
