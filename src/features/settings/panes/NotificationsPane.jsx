import * as React from "react";
import { Switch } from "@/components/ui/switch";
import { useNotificationSettings } from "@/hooks/use-notification-settings";
import { SettingsRow, SettingsSection } from "../SettingsRow";

/**
 * What Inertia interrupts you for.
 *
 * There are two questions here and they are deliberately separate sections,
 * because they fail in different ways. **What** is worth saying is a judgement
 * about the work - a turn that failed matters more than a turn that finished,
 * and a turn that handed the floor to the next agent has not finished anything
 * at all. **How** it is said is a judgement about attention: a banner is right
 * when the window is behind a browser and is pure noise when it is not.
 *
 * Two rules hold whatever is set here, and they are not switches because there
 * is no reading of them that is ever wanted:
 *
 *   · nothing is announced while the window is focused - the reply is on
 *     screen, in front of you;
 *   · a turn you cancelled is never announced, because you are the one who
 *     just cancelled it.
 */

const WHAT = [
  {
    key: "finished",
    label: "When a turn finishes",
    description:
      "A four-minute turn that ended while you were in another window. Silent if you were watching it happen.",
  },
  {
    key: "failed",
    label: "When a turn fails",
    description:
      "The provider refused, the key expired, a tool broke. The one most people keep on after turning the rest off.",
  },
  {
    key: "waiting",
    label: "When something is waiting on you",
    description:
      "A permission card or a question. This is the only kind that will never resolve itself: the turn is stopped until you answer.",
  },
  {
    key: "chained",
    label: "When one agent hands over to the next",
    description:
      "Off by default. A room answering a single question is several turns, and every one but the last ends by passing the floor on - announcing them is a notice per sentence.",
  },
];

const HOW = [
  {
    key: "banners",
    label: "Desktop banners",
    description:
      "A notification from the operating system, raised only when no Inertia window has focus.",
  },
  {
    key: "toasts",
    label: "In-app toasts",
    description:
      "The card in the corner of the window. Every notice draws one, including the ones that also became a banner, so coming back to the app shows what you missed rather than nothing at all.",
  },
];

function Rows({ items, choices, loading, onChange }) {
  return items.map((item) => (
    <SettingsRow
      key={item.key}
      label={item.label}
      htmlFor={`notify-${item.key}`}
      description={item.description}
      control={
        <Switch
          id={`notify-${item.key}`}
          size="sm"
          label={item.label}
          disabled={loading}
          checked={!!choices[item.key]}
          onCheckedChange={(value) => onChange(item.key, value)}
        />
      }
    />
  ));
}

export function NotificationsPane() {
  const { available, loading, choices, set } = useNotificationSettings();

  // No bridge means no half of the app that raises notices, so there is nothing
  // these switches could change. Said plainly rather than drawn as six dead
  // controls.
  if (!available) {
    return (
      <SettingsSection
        flat
        title="Notifications"
        description="This build has no notification service to configure."
      />
    );
  }

  const quiet = !choices.banners && !choices.toasts;

  return (
    <>
      <SettingsSection
        title="What you are told about"
        description="Nothing here fires while you are looking at the window, and a turn you cancelled is never announced."
      >
        <Rows items={WHAT} choices={choices} loading={loading} onChange={set} />
      </SettingsSection>

      <SettingsSection
        title="How you are told"
        description={
          quiet
            ? "Both channels are off, so nothing will reach you anywhere - including a turn that is blocked waiting for your answer."
            : "A banner is for when the window is behind something else. A toast is the record of it, in the app."
        }
      >
        <Rows items={HOW} choices={choices} loading={loading} onChange={set} />
      </SettingsSection>
    </>
  );
}
