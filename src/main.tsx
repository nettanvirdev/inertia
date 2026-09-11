import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/globals.css";
import { applyTheme } from "./lib/prefs";

// The stored theme can only be read asynchronously (it lives in a file the
// Rust side owns), so paint something synchronously first or the window shows
// one unstyled frame before the real preference lands. Dark rather than the OS
// preference, because dark is also the default preference - guessing "system"
// here would flash light on a light machine for every user who never changed
// the setting. `App` overwrites this as soon as preferences load.
applyTheme("dark");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
