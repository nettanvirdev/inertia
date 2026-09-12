import * as React from "react";
import { useToast } from "@/components/ui/toast";

/**
 * The window's half of being told something.
 *
 * Main decides what is worth interrupting for and whether the operating system
 * should hear about it; this decides how it looks in the app. The split matters
 * because only main can answer "is anyone looking at this window", and only the
 * window can draw a toast next to the thing it is about.
 *
 * Every notice arrives here, including the ones that also became an OS banner.
 * That is deliberate: someone who was away comes back to a toast still on
 * screen, rather than to a banner they have already dismissed and no trace of
 * it anywhere in the app.
 */

const bridge = () => (typeof window !== "undefined" ? window.notifyAPI : null) ?? null;

/**
 * Subscribe once, at the root.
 *
 * Not per view. A notice about a team that finished has to arrive whether the
 * person is looking at the chat, the computers screen or the settings dialog -
 * which is precisely when it is worth sending.
 */
export function useNotifications() {
  const { toast } = useToast();

  React.useEffect(() => {
    const api = bridge();
    if (!api?.onEvent) return undefined;
    return api.onEvent((event) => {
      if (!event?.title) return;
      toast({
        title: event.title,
        description: event.body,
        variant: event.tone === "problem" ? "danger" : "success",
        // Longer than the default four seconds. These are about work that ran
        // while the person was elsewhere, so the useful case is the one where
        // they are only now turning back to the screen.
        duration: event.tone === "problem" ? 10_000 : 6000,
      });
    });
  }, [toast]);
}
