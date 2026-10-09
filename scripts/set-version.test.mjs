import { describe, expect, it } from "vitest";
import { cargoVersion, isSemver, jsonVersion, withCargoVersion, withJsonVersion } from "./set-version.mjs";

describe("set-version", () => {
  it("valida versiones semánticas", () => {
    for (const ok of ["0.1.0", "1.20.3", "2.0.0-beta.1"]) expect(isSemver(ok), ok).toBe(true);
    for (const bad of ["1.0", "v1.0.0", "a.b.c", "1.0.0.0", ""]) expect(isSemver(bad), bad).toBe(false);
  });

  it("cambia solo la versión del paquete en Cargo.toml, no la de las dependencias", () => {
    const toml = `[package]\nname = "x"\nversion = "0.1.0"\n\n[dependencies]\nfoo = { version = "1.2.3" }\nbar = "2"\n`;
    const out = withCargoVersion(toml, "0.2.0");
    expect(cargoVersion(out)).toBe("0.2.0");
    expect(out).toContain('foo = { version = "1.2.3" }');
  });

  it("cambia la primera versión de un JSON (la del propio paquete)", () => {
    const json = `{\n  "name": "x",\n  "version": "0.1.0",\n  "dependencies": { "a": "^1" }\n}\n`;
    const out = withJsonVersion(json, "1.0.0");
    expect(jsonVersion(out)).toBe("1.0.0");
    expect(out).toContain('"a": "^1"');
  });
});
