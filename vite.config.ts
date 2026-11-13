import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// https://vite.dev/config/ and https://vitest.dev/config/
export default defineConfig({
  plugins: [react()],
  // Keep Rust compiler output visible when Vite runs under `tauri dev`.
  clearScreen: false,
  server: {
    // Must match `build.devUrl` in src-tauri/tauri.conf.json.
    port: 1420,
    strictPort: true,
    watch: {
      // The repo root is also a Cargo workspace; do not watch Rust files or build output.
      ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"],
    },
  },
  build: {
    // The app runs in WKWebView on macOS 14.4 or later, which is Safari 17.4.
    target: "safari17",
    // One file, read from disk inside the app, so its size costs little. The
    // editor (CodeMirror) is most of it: about 560 kB with it, 250 kB without.
    chunkSizeWarningLimit: 700,
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["./src/test/setup.ts"],
  },
});
