import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
// @ts-expect-error type error without @types/node package
import path from "node:path";
const host = process.env.TAURI_DEV_HOST;

// @ts-expect-error type error without @types/node package
const here = import.meta.dirname;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react(), tailwindcss()],

  resolve: {
    alias: {
      // The two aliases `src/` is written against. `@shared` is pure ESM with
      // no imports of its own: the rules several screens must give the same
      // answer to, several of them twinned on the Rust side, testable without
      // a DOM.
      "@": path.resolve(here, "./src"),
      "@shared": path.resolve(here, "./src/shared"),
    },
  },

  build: {
    target: "chrome130",
    // Only for debug builds (`tauri build --debug` sets TAURI_ENV_DEBUG): a
    // release build embeds every byte of dist/ in the executable, and the maps
    // would add megabytes to every download for a stack trace nobody reads.
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    chunkSizeWarningLimit: 1500,
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },

  test: {
    include: ["src/**/*.test.{js,jsx}"],
    environment: "node",
  },
}));
