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
      // The two aliases the ported renderer is written against. `@shared` is
      // pure ESM with no imports of its own, which is why the same files can
      // back both halves of the app rather than growing a second answer to
      // the same question.
      "@": path.resolve(here, "./src"),
      "@shared": path.resolve(here, "./src/shared"),
    },
  },

  build: {
    target: "chrome130",
    // On, because a stack trace from a packaged build is otherwise a list of
    // minified names in one line of one file. The maps are only read when
    // devtools are open, so they cost disk and nothing at runtime.
    sourcemap: true,
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
