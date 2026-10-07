import { composio } from "@/lib/integrations";
import { voice } from "@/lib/voice";
import { computers } from "@/lib/computers";

/**
 * The keys the app needs by name.
 *
 * These are not ordinary secrets. The backend looks each one up by the
 * exact string below before the user has named anything, so renaming one would
 * silently disconnect the feature it powers, and deleting one is a thing to do
 * on purpose from the feature's own screen rather than by reaching for a bin
 * icon in a list. That is the whole reason they are a separate section: same
 * row, same reveal, same copy, two fewer ways to break something.
 *
 * `test` proves the key works rather than that a string is stored, which is the
 * one thing a list of names cannot tell you. A key pasted with a character
 * missing looks exactly like a key that works until something asks it a
 * question.
 */

const NUMBER = new Intl.NumberFormat();

export const REQUIRED_SECRETS = [
  {
    name: "COMPOSIO_API_KEY",
    title: "Composio",
    label: "Composio API key",
    icon: "Blocks",
    purpose: "Connects Gmail, Linear and the rest of the app catalogue.",
    url: "https://composio.dev",
    urlLabel: "Get a key at composio.dev",
    async test() {
      if (!(await composio.configured())) {
        return { ok: false, message: "No key is stored, so there is nothing to test." };
      }
      const { items } = await composio.toolkits({});
      return {
        ok: true,
        message: `Composio accepted the key and offered ${items.length} connectable apps.`,
      };
    },
  },
  {
    name: "ELEVENLABS_API_KEY",
    title: "ElevenLabs",
    label: "ElevenLabs API key",
    icon: "AudioLines",
    purpose: "Powers voice mode: dictation into the composer, and replies read aloud.",
    url: "https://elevenlabs.io/app/settings/api-keys",
    urlLabel: "Get a key at elevenlabs.io",
    async test() {
      if (!(await voice.configured())) {
        return { ok: false, message: "No key is stored, so there is nothing to test." };
      }
      const account = await voice.test();
      // The quota is the other thing a person wants to know at this moment, and
      // it costs nothing extra to say it.
      return {
        ok: true,
        message: `ElevenLabs accepted the key. On the ${account.tier} plan, with ${NUMBER.format(
          account.remaining
        )} of ${NUMBER.format(account.limit)} characters left.`,
      };
    },
  },
  {
    name: "DAYTONA_API_KEY",
    title: "Daytona",
    label: "Daytona API key",
    icon: "Cloud",
    purpose: "Runs an agent's computer in the cloud, so it keeps working when Inertia is closed.",
    url: "https://app.daytona.io",
    urlLabel: "Get a key at app.daytona.io",
    async test() {
      // Asked of the provider rather than of a bare endpoint, so what is tested
      // is the same call the computers screen makes - including the endpoint
      // override, which is the setting most likely to be wrong.
      const rows = await computers.providers();
      const daytona = rows.find((row) => row.id === "daytona");
      if (!daytona) return { ok: false, message: "The computers bridge did not answer." };
      return daytona.ok
        ? { ok: true, message: `Daytona accepted the key at ${daytona.apiUrl}.` }
        : { ok: false, message: daytona.reason };
    },
  },
];

export const REQUIRED_NAMES = REQUIRED_SECRETS.map((entry) => entry.name);
