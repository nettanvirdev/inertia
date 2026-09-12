// First, and deliberately so. This puts the `window.*API` namespaces the whole
// renderer is written against onto the window, backed by Tauri commands. Two
// modules below read their bridge while they evaluate rather than per call, so
// an import ordered after `App` would leave them holding `undefined` for the
// life of the process. This is the only line in `src/` that is not a verbatim
// copy of the Electron renderer.
import "@/bridge";

import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ThemeProvider, bootstrapTheme } from "@/lib/theme";
import { bootstrapAppearance } from "@/lib/appearance";
import { ToastProvider } from "@/components/ui/toast";

/*
 * The typefaces, bundled rather than hoped for.
 *
 * Three of the faces the appearance settings offer - Inter, JetBrains Mono,
 * and anything that has to set বাংলা - were names in a CSS stack and nothing
 * else: picking them did something only on a machine that already had the font
 * installed, and silently nothing everywhere else. Bengali was worse than
 * nothing, because every stack fell through to whatever the platform happened
 * to have, so the same sentence was drawn in a different face depending on
 * which Latin font was selected.
 *
 * These are variable subsets: one file per script per family, and the browser
 * fetches only the ranges a page actually uses. The Bengali faces are the
 * expensive ones and they are still worth it - a reading face for a script is
 * not a preference, it is whether the text is legible.
 */
import "@fontsource-variable/inter/wght.css";
import "@fontsource-variable/source-serif-4/wght.css";
import "@fontsource-variable/jetbrains-mono/wght.css";
import "@fontsource-variable/noto-sans-bengali/wght.css";
import "@fontsource-variable/noto-serif-bengali/wght.css";
import "@fontsource-variable/anek-bangla/wght.css";

import "./styles/globals.css";

// Both run before the first React paint - the CSP forbids an inline <script>.
// Order matters: the accent ships a light and a dark value, so the appearance
// cannot be painted until the theme it belongs to is on the document.
bootstrapTheme();
bootstrapAppearance();

ReactDOM.createRoot(document.getElementById("root")).render(
  <React.StrictMode>
    <ThemeProvider>
      <ToastProvider>
        <App />
      </ToastProvider>
    </ThemeProvider>
  </React.StrictMode>
);
