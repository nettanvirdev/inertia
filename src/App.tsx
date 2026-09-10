import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Titlebar } from "@/components/Titlebar";

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");

  async function greet() {
    setGreetMsg(await invoke("greet", { name }));
  }

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background text-foreground">
      <Titlebar title="inertia" />

      <main className="flex flex-1 items-center justify-center overflow-auto p-8">
        <div className="w-full max-w-sm card-surface rounded-lg p-6 shadow-md">
          <h1 className="text-lg font-semibold text-heading">Welcome to Tauri + React</h1>
          <p className="mt-1.5 text-sm text-muted-foreground">
            A themed starting point for the Inertia stack.
          </p>

          <form
            className="mt-5 flex gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              void greet();
            }}
          >
            <input
              id="greet-input"
              value={name}
              onChange={(e) => setName(e.currentTarget.value)}
              placeholder="Enter a name..."
              className="h-9 flex-1 rounded-full border border-input-border bg-input px-4 text-sm text-input-foreground outline-none placeholder:text-input-placeholder focus-visible:bg-input-focus"
            />
            <button
              type="submit"
              className="h-9 shrink-0 rounded-full accent-fill px-4 text-sm font-medium transition-opacity duration-150 ease-out hover:opacity-80 active:opacity-90"
            >
              Greet
            </button>
          </form>

          {greetMsg ? <p className="mt-4 text-sm text-foreground-secondary">{greetMsg}</p> : null}
        </div>
      </main>
    </div>
  );
}

export default App;
