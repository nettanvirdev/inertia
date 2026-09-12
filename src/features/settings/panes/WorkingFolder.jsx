import * as React from "react";
import { FolderOpen, FolderTree } from "@/components/icons";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useWorkspace } from "@/lib/workspace";
import { tools } from "@/lib/agent";
import { SettingsCard, SettingsRow, SettingsSection } from "../SettingsRow";

/**
 * Where agents actually work.
 *
 * This is not the Inertia folder and confusing the two would be a serious
 * mistake, so the copy says it plainly. The Inertia folder holds settings,
 * conversations and skills - the app's own state. The working folder is the
 * project an agent reads and edits, and pointing the file tools at the
 * workspace would mean an agent rewriting the app's own configuration while
 * answering a question about something else.
 *
 * Until this is set, agents work in `files/work` inside the workspace: an empty
 * folder that belongs to them. It used to be the user's home directory, which
 * was described here as safe. It was not - a home directory holds `.ssh`, the
 * cloud CLIs' credentials and every browser profile, and the file tools only
 * ask before leaving the working folder, so all of it was inside the boundary.
 */
export function WorkingFolder() {
  const { client, configured, native, browse } = useWorkspace();
  const [value, setValue] = React.useState("");
  const [saved, setSaved] = React.useState("");
  const [busy, setBusy] = React.useState(false);

  React.useEffect(() => {
    if (!configured) return;
    let alive = true;
    client
      .readDocument("settings.app", {})
      .then((doc) => {
        if (!alive) return;
        setValue(doc?.workingDirectory ?? "");
        setSaved(doc?.workingDirectory ?? "");
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [client, configured]);

  const save = React.useCallback(
    async (next) => {
      setBusy(true);
      try {
        const doc = await client.readDocument("settings.app", {});
        await client.writeDocument("settings.app", { ...doc, workingDirectory: next });
        setSaved(next);
      } finally {
        setBusy(false);
      }
    },
    [client]
  );

  const pick = async () => {
    const dir = await browse();
    if (!dir) return;
    setValue(dir);
    await save(dir);
  };

  const dirty = value !== saved;

  return (
    <SettingsSection flat title="Working folder">
      <SettingsCard className="flex flex-col gap-3">
        <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
          The project your agents read and edit. This is deliberately not the Inertia folder: that
          one holds your settings, conversations and skills, and an agent editing it while answering
          a question about something else is not a thing anyone wants. An agent can override this
          with a folder of its own.
        </p>

        <div className="flex items-center gap-2">
          <Input
            value={value}
            spellCheck={false}
            placeholder={native ? "Choose a folder" : "Set this in the desktop app"}
            onChange={(event) => setValue(event.target.value)}
            onBlur={() => dirty && save(value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && dirty) save(value);
            }}
            className="font-mono text-[12px]"
          />
          <Button variant="subtle" size="sm" disabled={!native || busy} onClick={pick}>
            <FolderTree />
            Choose
          </Button>
        </div>

        {/* the card supplies the inset here, so the row must not add its own */}
        <SettingsRow
          className="p-0"
          label="Open it"
          description={
            saved
              ? "Show the working folder in your file manager."
              : "Nothing is set, so agents work in files/work inside your workspace."
          }
          control={
            <Button
              variant="subtle"
              size="xs"
              disabled={!native || !saved}
              onClick={() => tools.openPath(saved)?.catch?.(() => {})}
            >
              <FolderOpen />
              Open folder
            </Button>
          }
        />
      </SettingsCard>
    </SettingsSection>
  );
}
