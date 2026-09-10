import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/globals.css";

// The theme is authored as explicit .light/.dark classes rather than a media
// query, so a future settings screen can offer more palettes than "system"
// (see the light/dark variants ported from the source design system). Until
// that screen exists, follow the OS preference so the app isn't stuck in
// light mode on a dark machine.
const media = window.matchMedia("(prefers-color-scheme: dark)");
const applyTheme = (isDark: boolean) => {
  document.documentElement.classList.toggle("dark", isDark);
};
applyTheme(media.matches);
media.addEventListener("change", (e) => applyTheme(e.matches));

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
