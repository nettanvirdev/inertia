import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Setup } from "@setup/Setup";
import { Uninstall } from "@setup/Uninstall";

type Mode = "install" | "uninstall";

/**
 * One frontend bundle, two binaries. `inertia-setup.exe` and
 * `inertia-uninstall.exe` are separate Tauri apps with separate Rust crates,
 * but they point at the same `frontendDist` and ask the backend which one they
 * are. Sharing the bundle means the title bar, the mark, the palette and the
 * button styling cannot drift between installing and uninstalling - which is
 * exactly the drift that leaves most apps with a beautiful installer and a
 * 1998 uninstaller.
 */
export function App() {
  const [mode, setMode] = useState<Mode | null>(null);

  useEffect(() => {
    void invoke<Mode>("app_mode")
      // In a plain browser there is no backend to ask. Default to the setup
      // screen; `?mode=uninstall` reaches the other one for design work.
      .catch<Mode>(() =>
        new URLSearchParams(location.search).get("mode") === "uninstall"
          ? "uninstall"
          : "install",
      )
      .then(setMode);
  }, []);

  if (mode === null) return <div className="h-screen bg-background" />;
  return mode === "uninstall" ? <Uninstall /> : <Setup />;
}
