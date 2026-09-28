/// <reference types="vitest/config" />
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  // Keep Rust compiler output visible when Vite runs under `tauri dev`.
  clearScreen: false,
  server: {
    // Must match `build.devUrl` in src-tauri/tauri.conf.json.
    port: 1420,
    strictPort: true,
    // Bound to localhost only: the dev server must not be reachable from the network.
    host: "localhost",
    watch: {
      ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"],
    },
  },
  build: {
    // Assets load from the app bundle on disk, not over a network, so bundle size
    // is not a latency concern. The ~1.1 MB main chunk is xterm.js, CodeMirror's
    // core and React; language grammars are separate chunks loaded on demand.
    chunkSizeWarningLimit: 1536,
  },
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
  },
});
