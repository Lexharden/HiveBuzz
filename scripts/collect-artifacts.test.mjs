import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { describe, expect, it } from "vitest";
import { bundleRoots, findArtifacts, isArtifact, sha256 } from "./collect-artifacts.mjs";

function fakeTauri() {
  const dir = mkdtempSync(join(tmpdir(), "hb-art-"));
  const bundle = join(dir, "target", "release", "bundle");
  for (const [sub, files] of Object.entries({ nsis: ["HiveBuzz_1.0.0_x64-setup.exe", "HiveBuzz_1.0.0_x64-setup.exe.sig", "notas.txt"], msi: ["HiveBuzz.msi"], macos: ["HiveBuzz.app"], deb: ["hivebuzz.deb"] })) {
    mkdirSync(join(bundle, sub), { recursive: true });
    for (const f of files) writeFileSync(join(bundle, sub, f), "x");
  }
  // Compilación con --target: otra carpeta bundle.
  const cross = join(dir, "target", "aarch64-apple-darwin", "release", "bundle", "dmg");
  mkdirSync(cross, { recursive: true });
  writeFileSync(join(cross, "HiveBuzz_aarch64.dmg"), "x");
  return dir;
}

describe("collect-artifacts", () => {
  it("reconoce instaladores y firmas, sin importar mayúsculas", () => {
    for (const f of ["a.exe", "A.MSI", "x.dmg", "x.AppImage", "x.deb", "x.rpm", "x.exe.sig"]) expect(isArtifact(f), f).toBe(true);
    for (const f of ["notas.txt", "x.zip", "x.app", "exe", ""]) expect(isArtifact(f), f).toBe(false);
  });

  it("encuentra solo los instaladores, también los de una compilación con --target", () => {
    const dir = fakeTauri();
    const roots = bundleRoots(dir);
    expect(roots).toHaveLength(2);
    const names = roots.flatMap(findArtifacts).map((p) => basename(p)).sort();
    expect(names).toEqual(["HiveBuzz.msi", "HiveBuzz_1.0.0_x64-setup.exe", "HiveBuzz_1.0.0_x64-setup.exe.sig", "HiveBuzz_aarch64.dmg", "hivebuzz.deb"]);
  });

  it("una carpeta sin compilar no da error ni resultados", () => {
    const empty = mkdtempSync(join(tmpdir(), "hb-art-"));
    expect(bundleRoots(empty).flatMap(findArtifacts)).toEqual([]);
  });

  it("calcula SHA-256", () => {
    const f = join(mkdtempSync(join(tmpdir(), "hb-art-")), "a.txt");
    writeFileSync(f, "abc");
    expect(sha256(f)).toBe("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
  });
});
