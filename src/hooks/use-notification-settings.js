import * as React from "react";

/**
 * What the person agreed to be told about.
 *
 * Not part of the preferences store, for the same reason the background
 * switches are not: the half of the app that decides whether a notice is worth
 * raising runs outside the window, and it reads these from `settings.app`
 * directly. A renderer copy could disagree with the one actually consulted,
 * which would show up as switches that look right and change nothing.
 *
 * `available` is false in a browser tab and in tests, where there is no bridge
 * at all. The pane hides the rows rather than drawing dead controls.
 */

/** What the app does before anything has been stored. Twin of `notify::Choices`. */
export const NOTIFICATION_DEFAULTS = {
  finished: true,
  failed: true,
  waiting: true,
  // The one that is noise by construction: a turn that hands the floor to the
  // next agent has finished a sentence, not the work.
  chained: false,
  banners: true,
  toasts: true,
};

export function useNotificationSettings() {
  const api = typeof window === "undefined" ? null : window.notifyAPI;
  const supported = !!api?.choices;
  const [state, setState] = React.useState(null);
  const [loading, setLoading] = React.useState(supported);

  const reload = React.useCallback(async () => {
    if (!supported) return;
    setLoading(true);
    try {
      setState(await api.choices());
    } catch {
      setState(null);
    } finally {
      setLoading(false);
    }
  }, [api, supported]);

  React.useEffect(() => {
    reload();
  }, [reload]);

  /**
   * Flip one switch and take the app's word for the result.
   *
   * Not optimistic: a workspace that has not been chosen yet has nowhere to
   * store this, and a switch that slides across and means nothing is worse
   * than one that takes a moment.
   */
  const set = React.useCallback(
    async (key, value) => {
      if (!supported) return;
      try {
        setState(await api.setChoices({ [key]: !!value }));
      } catch {
        await reload();
      }
    },
    [api, reload, supported]
  );

  return {
    available: supported,
    loading,
    choices: { ...NOTIFICATION_DEFAULTS, ...(state ?? {}) },
    set,
    reload,
  };
}
