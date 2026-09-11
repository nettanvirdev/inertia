import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
// @ts-expect-error type error without @types/node package
import path from "node:path";

const root = path.resolve(import.meta.dirname, "./installer");

/**
 * The setup bootstrapper's frontend. A separate Vite project rather than a
 * second entry in the main config: it ships as its own binary with its own
 * Tauri shell, and nothing in the app's bundle should be able to reach it.
 * The `@` alias still points at the app's `src/`, which is the whole point -
 * the installer draws from the same icon set and the same theme tokens.
 */
export default defineConfig(() => ({
  root,
  plugins: [react(), tailwindcss()],

  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "./src"),
      "@setup": path.resolve(root, "./src"),
    },
  },

  build: {
    outDir: path.resolve(import.meta.dirname, "./dist-installer"),
    emptyOutDir: true,
  },

  clearScreen: false,
  server: {
    // 1420 belongs to the app's dev server; the two are often up together.
    port: 1430,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
