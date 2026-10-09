// Punto de entrada del sidecar. stdout es EXCLUSIVO para NDJSON hacia Rust;
// todo lo demás (incluido lo que escriba la librería) va a stderr.

import { createInterface } from "node:readline";
import { Connector } from "./connector";
import { encode, parseCommand, type SidecarMessage } from "./protocol";
import { createTikTokClient } from "./tiktok";

const toStderr = (...args: unknown[]) => process.stderr.write(args.map(String).join(" ") + "\n");
console.log = toStderr;
console.info = toStderr;
console.debug = toStderr;

const emit = (msg: SidecarMessage) => {
  process.stdout.write(encode(msg));
};

const connector = new Connector({ factory: createTikTokClient, emit });

async function shutdown(code: number): Promise<never> {
  await Promise.race([connector.disconnect(), new Promise((r) => setTimeout(r, 3_000))]);
  process.exit(code);
}

// Un error inesperado nunca debe dejar al sidecar callado: se informa y Rust decide.
process.on("uncaughtException", (err) => {
  emit({ kind: "log", level: "error", message: `uncaughtException: ${err.message}` });
});
process.on("unhandledRejection", (reason) => {
  emit({ kind: "log", level: "error", message: `unhandledRejection: ${String(reason)}` });
});

const rl = createInterface({ input: process.stdin });

rl.on("line", (line) => {
  if (line.trim() === "") return;
  const cmd = parseCommand(line);
  if (!cmd) {
    emit({ kind: "log", level: "warn", message: `comando inválido ignorado: ${line.slice(0, 200)}` });
    return;
  }
  switch (cmd.cmd) {
    case "connect":
      void connector.connect({
        uniqueId: cmd.uniqueId,
        ...(cmd.eulerApiKey ? { eulerApiKey: cmd.eulerApiKey } : {}),
        ...(cmd.session ? { session: cmd.session } : {}),
      });
      break;
    case "disconnect":
      void connector.disconnect();
      break;
    case "sendChat":
      void connector.sendChat(cmd.requestId, cmd.text);
      break;
    case "shutdown":
      void shutdown(0);
      break;
  }
});

// Si Rust cierra el pipe (o muere), el sidecar no debe quedar huérfano.
rl.on("close", () => void shutdown(0));

emit({ kind: "ready" });
