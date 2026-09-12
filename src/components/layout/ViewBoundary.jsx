import * as React from "react";
import { ShieldAlert } from "@/components/icons";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";

/**
 * The wall between one screen and the window.
 *
 * Every hardened read in this app is a guess about which field a record might be
 * missing, and the guesses are only as good as the shapes we thought of. This is
 * the part that does not have to guess: a screen that throws loses the screen,
 * not the rail, the titlebar and the user's way back to a screen that works.
 *
 * The bound is one view rather than the whole shell on purpose. Wrapping `App`
 * would catch the same errors and still leave the user staring at a page with no
 * navigation on it - which is the blank window again, just with an apology
 * printed on it.
 *
 * Recovery is genuinely two different things, so it offers both. "Try again"
 * remounts the view, which is the right answer when the failure was in a render
 * that a state change has since made moot. Switching views in the rail resets
 * this boundary too - `Shell` keys it on the view name - so a broken screen
 * never latches the app into its error state.
 *
 * A class component because `componentDidCatch` has no hook equivalent; React
 * still has no other way to catch a render error.
 */
export class ViewBoundary extends React.Component {
  constructor(props) {
    super(props);
    this.state = { error: null };
    this.retry = () => this.setState({ error: null });
  }

  static getDerivedStateFromError(error) {
    return { error };
  }

  componentDidCatch(error, info) {
    // The devtools console is where a desktop app's stack traces belong; there
    // is no server to send this to and inventing one is not this file's job.
    console.error(`[${this.props.name ?? "view"}] render failed`, error, info?.componentStack);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="flex min-h-0 flex-1 items-center justify-center">
        <EmptyState
          icon={ShieldAlert}
          title="This screen stopped"
          // Named, because "something went wrong" tells a user nothing they can
          // act on, and the folder is a place they can actually go and look.
          description={`${
            this.props.label ?? "The view"
          } could not be drawn - usually a record in the workspace folder that is missing a field. The rest of the app is unaffected.`}
          action={
            <div className="flex flex-col items-center gap-2">
              <Button size="sm" onClick={this.retry}>
                Try again
              </Button>
              <code className="max-w-96 truncate text-[11px] text-muted-foreground">
                {String(error?.message ?? error)}
              </code>
            </div>
          }
        />
      </div>
    );
  }
}
