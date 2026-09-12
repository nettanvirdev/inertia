import { raw, subscribe } from "./envelope";

/**
 * Being told something, when the app decided it was worth interrupting for.
 *
 * A feed and nothing else. The decision - is this worth saying, and does it
 * also deserve an operating-system banner - belongs to the app process, which
 * is the only half that can answer "is anybody looking at this window right
 * now". This end draws the toast.
 *
 * Every notice arrives here, including the ones that also became a banner.
 * That is deliberate: somebody who was away comes back to a toast still on
 * screen, rather than to a banner they have already dismissed and no trace of
 * it anywhere in the app.
 *
 * The payload is `{ title, body, tone, meta, at, focused }`, and `lib/notify.js`
 * reads the first three.
 *
 * The two settings calls are raw rather than enveloped, and answer the whole
 * choices object rather than an acknowledgement, for the same reason the
 * background switches do: the pane shows what was actually stored, so a write
 * that could not land shows as a switch that did not move.
 */
export function notifyBridge() {
  return {
    /** Subscribe once, at the root. Returns its own unsubscribe. */
    onEvent: (callback) => subscribe("notify:event", callback),
    /** What the person agreed to be told about. */
    choices: () => raw("notifications_settings"),
    /** Change some of them. Fields left out keep their stored value. */
    setChoices: (value) => raw("notifications_set", { value: value ?? {} }),
  };
}
