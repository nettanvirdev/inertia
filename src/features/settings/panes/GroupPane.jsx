import * as React from "react";
import { useWorkspace } from "@/lib/workspace";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { SettingsRow, SettingsSection } from "../SettingsRow";
import { DEFAULT_GROUP } from "@shared/group";

/**
 * How the agents in a group conversation work together.
 *
 * Two kinds of setting, and the difference is worth keeping visible. The
 * instructions are words the person writes and every agent in a room reads -
 * house style, what to argue about, when to come back and ask. The permissions
 * are structural: whether an agent may change who is in the room at all.
 *
 * The limits are on the same screen because they are the cost dial. Five agents
 * that can each call the next have no natural end, and this is where a person
 * decides how long they are willing to let that run before being asked.
 */

const DOC = "settings.group";

/**
 * What the empty field suggests.
 *
 * Written as three separate instructions rather than one sentence, because the
 * shape of the example is the real hint: this is a list of house rules, not a
 * description of a personality.
 */
const PLACEHOLDER = [
  "Disagree early rather than politely, and say what the trade-off is.",
  "Settle anything you can between yourselves. Bring me in for decisions about what I actually want.",
  "Ask before anything that costs money.",
].join("\n");

export function GroupPane() {
  const { client, configured } = useWorkspace();
  const [doc, setDoc] = React.useState(null);

  React.useEffect(() => {
    if (!configured) return;
    let alive = true;
    client
      .readDocument(DOC, {})
      .then((stored) => alive && setDoc(stored ?? {}))
      .catch(() => alive && setDoc({}));
    return () => {
      alive = false;
    };
  }, [client, configured]);

  const permissions = { ...DEFAULT_GROUP.permissions, ...(doc?.permissions ?? {}) };
  const instructions = doc?.prompt ?? "";

  /**
   * Written on every change rather than behind a Save button.
   *
   * The document is small and the read is already in hand, so the round trip
   * costs nothing a person would notice - and a settings screen that can be
   * left in an unsaved state is a settings screen that lies about what the app
   * is doing.
   */
  const save = React.useCallback(
    async (patch) => {
      setDoc((current) => ({ ...current, ...patch }));
      const stored = (await client.readDocument(DOC, {})) ?? {};
      await client.writeDocument(DOC, { ...stored, ...patch });
    },
    [client]
  );

  const setPermission = (key, value) => save({ permissions: { ...permissions, [key]: value } });

  return (
    <div>
      <SettingsSection
        title="How they work together"
        description="Every agent in a group conversation reads this, alongside its own instructions. Say how you want them to collaborate - what to argue about, what to settle between themselves, and when to come back and ask you."
      >
        {/* Not a SettingsRow: this is not a label with a control beside it, it
            is a page of writing. And not `autoResize` either - that sizes a box
            to what is in it, which is right for a composer and wrong here,
            where an empty field would collapse to one line and read as a
            search box rather than as somewhere to write a paragraph. */}
        <div className="px-4 py-3">
          <Textarea
            value={instructions}
            rows={9}
            spellCheck={false}
            onChange={(event) => save({ prompt: event.target.value })}
            placeholder={PLACEHOLDER}
            aria-label="How the agents should work together"
            className="leading-relaxed"
          />
          <p className="mt-2 text-[0.6875rem] leading-relaxed text-muted-foreground">
            {instructions.trim()
              ? "Your own instructions are in use."
              : "Nothing set: the agents follow the built-in rules for working together."}
          </p>
        </div>
      </SettingsSection>

      <SettingsSection
        title="What they may do on their own"
        description="A group conversation is autonomous by default: the agents decide who is needed and when they are done. Turn any of these off and that decision comes back to you - you can still bring anyone in yourself by naming them with @."
      >
        <SettingsRow
          label="Bring another agent in"
          description="An agent can add a colleague to the conversation when the work reaches them."
          control={
            <Switch
              checked={permissions.canInvite}
              onCheckedChange={(next) => setPermission("canInvite", next)}
              aria-label="Let agents bring another agent in"
            />
          }
        />
        <SettingsRow
          label="Hand the conversation over"
          description="An agent can give the conversation to someone else. The name and face at the top of the thread change with it."
          control={
            <Switch
              checked={permissions.canHandover}
              onCheckedChange={(next) => setPermission("canHandover", next)}
              aria-label="Let agents hand the conversation over"
            />
          }
        />
        <SettingsRow
          label="Leave when finished"
          description="An agent can step out once its part is done. The agent the conversation belongs to can never leave it."
          control={
            <Switch
              checked={permissions.canLeave}
              onCheckedChange={(next) => setPermission("canLeave", next)}
              aria-label="Let agents leave the conversation"
            />
          }
        />
      </SettingsSection>

      <SettingsSection
        title="How far they go without you"
        description="The two limits that decide what a group conversation can cost while you are not looking."
      >
        <SettingsRow
          label={`Agents in one conversation: ${permissions.maxAgents}`}
          description="Including the one it belongs to. Past this, an agent asking for a colleague is told the room is full."
          control={
            <Slider
              value={permissions.maxAgents}
              min={2}
              max={10}
              step={1}
              onChange={(next) => setPermission("maxAgents", next)}
              className="w-44"
              aria-label="Agents in one conversation"
            />
          }
        />
        <SettingsRow
          label={`Turns between your messages: ${permissions.maxHops}`}
          description="How many times the agents may speak to each other before the conversation comes back to you. Saying anything starts the count again."
          control={
            <Slider
              value={permissions.maxHops}
              min={1}
              max={20}
              step={1}
              onChange={(next) => setPermission("maxHops", next)}
              className="w-44"
              aria-label="Turns between your messages"
            />
          }
        />
      </SettingsSection>
    </div>
  );
}
