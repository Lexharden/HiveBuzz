// Carga `.env` (si existe), revisa lo importante y ejecuta el comando que sigue.
//
//   bun scripts/with-env.mjs tauri dev
//   bun scripts/with-env.mjs tauri build
//
// Sin dependencias. Lo que ya esté definido en el entorno (p. ej. en CI) manda sobre `.env`.

import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Lee un `.env` sencillo: CLAVE=valor, comentarios con #, comillas opcionales. */
export function parseEnv(text) {
  const out = {};
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    if (eq < 1) continue;
    const key = line.slice(0, eq).trim();
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) continue;
    let value = line.slice(eq + 1).trim();
    if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'"))) value = value.slice(1, -1);
    else value = value.replace(/\s+#.*$/, "");
    out[key] = value;
  }
  return out;
}

/** Problemas (bloqueantes) y avisos del entorno resultante. */
export function check(env, command) {
  const problems = [];
  const notes = [];
  const id = env.HIVEBUZZ_SPOTIFY_CLIENT_ID ?? "";
  if (id === "") notes.push("HIVEBUZZ_SPOTIFY_CLIENT_ID vacío: el botón «Conectar con Spotify» saldrá desactivado.");
  else if (!/^[A-Za-z0-9]{16,64}$/.test(id)) problems.push("HIVEBUZZ_SPOTIFY_CLIENT_ID no parece un Client ID de Spotify (letras y números, sin espacios).");

  const twitch = env.HIVEBUZZ_TWITCH_CLIENT_ID ?? "";
  if (twitch === "") notes.push("HIVEBUZZ_TWITCH_CLIENT_ID vacío: el chat de Twitch funciona, pero sin el botón «Entrar con Twitch».");
  else if (!/^[A-Za-z0-9]{16,64}$/.test(twitch)) problems.push("HIVEBUZZ_TWITCH_CLIENT_ID no parece un Client ID de Twitch (letras y números, sin espacios).");

  const keyPath = env.TAURI_SIGNING_PRIVATE_KEY_PATH ?? "";
  if (keyPath && !existsSync(keyPath)) problems.push(`TAURI_SIGNING_PRIVATE_KEY_PATH apunta a un archivo que no existe: ${keyPath}`);
  if (command.includes("build") && !keyPath && !env.TAURI_SIGNING_PRIVATE_KEY) notes.push("Sin clave de firma: el instalador se genera igual, pero sin artefactos de auto-actualización.");
  return { problems, notes };
}

function main() {
  const [cmd, ...args] = process.argv.slice(2);
  if (!cmd) {
    console.error("Uso: bun scripts/with-env.mjs <comando> [argumentos]");
    process.exit(2);
  }
  const file = join(root, ".env");
  let fromFile = {};
  if (existsSync(file)) fromFile = parseEnv(readFileSync(file, "utf8"));
  else console.warn("ℹ️  No hay .env (copia .env.example a .env). Se sigue con el entorno actual.");

  // Lo ya definido en el entorno gana, salvo que esté vacío.
  const env = { ...fromFile };
  for (const [k, v] of Object.entries(process.env)) if (v !== undefined && v !== "") env[k] = v;

  const { problems, notes } = check(env, [cmd, ...args].join(" "));
  for (const n of notes) console.warn(`ℹ️  ${n}`);
  if (problems.length > 0) {
    for (const p of problems) console.error(`❌ ${p}`);
    process.exit(1);
  }

  // Cargo no reconstruye por cambiar una variable de entorno salvo que se lo pidamos (ver build.rs).
  if (cmd === "--check") {
    console.log("✅ Entorno correcto.");
    process.exit(0);
  }

  const child = spawn(cmd, args, { cwd: root, env, stdio: "inherit", shell: process.platform === "win32" });
  child.on("exit", (code, signal) => process.exit(code ?? (signal ? 1 : 0)));
  child.on("error", (e) => {
    console.error(`No se pudo ejecutar «${cmd}»: ${e.message}`);
    process.exit(1);
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
