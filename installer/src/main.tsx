import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "@setup/App";
import "./styles/setup.css";

// The installer is dark, full stop - it does not follow the OS the way the app
// does. There is nothing installed yet to hold a preference, the window is on
// screen for under a minute, and the app it is installing starts dark too, so
// following a light OS here would just make setup the one light frame in the
// whole first-run experience.
document.documentElement.classList.add("dark");
document.documentElement.classList.remove("light");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
