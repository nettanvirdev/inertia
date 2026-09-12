import * as React from "react";
import { Check, Copy } from "@/components/icons";
import { IconButton } from "@/components/ui/icon-button";
import { Tooltip } from "@/components/ui/tooltip";
import { copyText } from "@/lib/clipboard";

/**
 * Copy, then say so on the button itself. A toast for one word is too loud, and
 * the tick lands where the eye already is.
 *
 * `value` covers the common case. `onCopy` is for a value nobody should hold in
 * React state until it is asked for - a secret is read from the workspace at the
 * moment the button is pressed, not when the row renders.
 */
export function CopyButton({ value, label = "Copy", size = "sm", onCopy }) {
  const [done, setDone] = React.useState(false);

  React.useEffect(() => {
    if (!done) return undefined;
    const timer = setTimeout(() => setDone(false), 1400);
    return () => clearTimeout(timer);
  }, [done]);

  async function copy() {
    // The tick is the whole feedback here, so it is shown only when the write
    // actually happened; a refused clipboard leaves the button as it was.
    if (await copyText(String((await onCopy?.()) ?? value ?? ""))) setDone(true);
  }

  return (
    <Tooltip content={done ? "Copied" : label}>
      <IconButton size={size} label={label} onClick={copy}>
        {done ? <Check /> : <Copy />}
      </IconButton>
    </Tooltip>
  );
}
