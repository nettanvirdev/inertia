import * as React from "react";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { isThreadWorking } from "@shared/transcript";
import { COMPUTER_STATUS_META } from "@/data/computers";
import { Dialog, DialogTitle } from "@/components/ui/dialog";
import { Composer } from "@/features/chat/Composer";
import { DesktopPane } from "./DesktopPane";

/**
 * The machine's screen, over the chat rather than instead of it.
 *
 * Before this, watching the agent work or taking the mouse from it meant
 * leaving the conversation for the Computers screen, and coming back to say
 * "no, the other button" meant leaving the machine. The two things a person
 * does while an agent drives a browser - watch it, and talk to it - were on
 * different pages.
 *
 * So the screen opens here, in a dialog the size of the settings one, and the
 * composer comes with it: the same one the chat uses, wired to the same
 * thread, so a message typed under the screen lands in the conversation
 * behind the dialog. Close it and the chat is exactly where it was. `mode`
 * decides whether it opens watching or driving; the pane's own button flips
 * that afterwards.
 */
export function ComputerDialog({ computer, thread, mode, open, onOpenChange }) {
  const { sendMessage, messages } = useApp();
  const controlling = mode === "control";

  // Whether the agent is mid-reply, for the composer's stop button - by the
  // same rule the chat uses, so the two cannot disagree about one thread.
  // Messages are kept per thread, not as one list.
  const streaming = React.useMemo(
    () => isThreadWorking(thread?.id ? messages?.[thread.id] : null),
    [messages, thread?.id]
  );

  if (!computer) return null;
  const status = COMPUTER_STATUS_META[computer.status] ?? COMPUTER_STATUS_META.stopped;

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      size="full"
      ariaLabel={`${computer.name}`}
      className="p-0"
    >
      <div className="flex h-full min-h-0 flex-col gap-3 p-5">
        <div className="flex shrink-0 items-center gap-3 pr-8">
          <DialogTitle className="truncate">{computer.name}</DialogTitle>
          <span className="inline-flex h-5 items-center gap-1.5 rounded-full fill-secondary px-2 text-[10.5px] text-muted-foreground">
            <span
              aria-hidden="true"
              className={cn(
                "size-1.5 rounded-full",
                computer.status === "running" && "animate-soft-pulse"
              )}
              style={{ backgroundColor: status.color }}
            />
            {status.label}
          </span>
          <p className="min-w-0 flex-1 truncate text-[12px] text-muted-foreground">
            {controlling
              ? "You opened this to take control. The agent is still working; say so below if you want it to wait."
              : "Watching. Take control to use the mouse and keyboard yourself."}
          </p>
        </div>

        <div className="flex min-h-0 flex-1 flex-col">
          <DesktopPane
            key={`${computer.id}:${mode}`}
            computer={computer}
            initialControlling={controlling}
          />
        </div>

        {thread ? (
          <div className="shrink-0">
            <Composer
              threadId={thread.id}
              agentId={thread.agentId}
              streaming={streaming}
              placeholder="Tell the agent what you see, or what to do next"
              onSend={(text, options) => sendMessage(thread.id, text, options)}
            />
          </div>
        ) : null}
      </div>
    </Dialog>
  );
}
