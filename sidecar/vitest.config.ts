import { defineConfig } from "vitest/config";

// Config propia: sin ella, Vite busca hacia arriba y heredaría la de la raíz (que es para los overlays).
export default defineConfig({
  test: {
    include: ["test/**/*.test.ts"],
    environment: "node",
  },
});
