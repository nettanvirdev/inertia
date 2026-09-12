import * as React from "react";
import { Avatar } from "@/components/ui/avatar";
import {
  Dialog,
  DialogBody,
  DialogDescription,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useToast } from "@/components/ui/toast";
import { useApp } from "@/lib/store";

/**
 * Who gets to use this machine.
 *
 * The assignment is what makes the `computer` tool work: an agent with one can
 * run commands somewhere that is not the user's laptop, and an agent without
 * one is not even offered the tool. So this dialog is not decoration - it is
 * the switch that hands a teammate a machine.
 *
 * One machine per agent, which is why picking this computer takes an agent off
 * whichever one it was on. An agent with two computers has no answer to "run
 * this somewhere", and the panes have nowhere to put the second.
 */
export function AssignAgentsDialog({ open, onOpenChange, computer, agents }) {
  const { assignComputer } = useApp();
  const { toast } = useToast();

  const [picked, setPicked] = React.useState(() => new Set(computer.assignedAgentIds ?? []));
  const [busy, setBusy] = React.useState(false);

  React.useEffect(() => {
    if (open) setPicked(new Set(computer.assignedAgentIds ?? []));
  }, [open, computer.assignedAgentIds]);

  function toggle(agentId) {
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(agentId)) next.delete(agentId);
      else next.add(agentId);
      return next;
    });
  }

  async function save() {
    setBusy(true);
    try {
      await assignComputer(computer.id, [...picked]);
      toast({
        variant: "success",
        title: "Assignment saved",
        description: `${picked.size} agent${picked.size === 1 ? "" : "s"} on ${computer.name}.`,
      });
      onOpenChange(false);
    } catch (error) {
      toast({ variant: "error", title: "Could not save it", description: error?.message });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange} size="md">
      <DialogTitle>Assign agents</DialogTitle>
      <DialogDescription>
        An agent assigned here can run commands and change files on {computer.name} with its
        computer tool. Its permission rules still decide whether a command runs at all.
      </DialogDescription>

      <DialogBody>
        <ScrollArea className="max-h-[min(24rem,calc(100dvh-20rem))] -mx-1 px-1">
          {agents.length === 0 ? (
            <p className="py-6 text-center text-[13px] text-muted-foreground">
              This workspace has no agents yet.
            </p>
          ) : (
            <div className="flex flex-col gap-1">
              {agents.map((agent) => {
                const elsewhere =
                  agent.computerId && agent.computerId !== computer.id ? agent.computerId : null;
                return (
                  /*
                    The row is the control, and the box is a picture of its state.
                    
                    Three things had to be true at once and only this shape gets
                    all three. The whole row should be clickable, because a 16px
                    target beside a name is a miss waiting to happen. `Checkbox`
                    is itself a <button>, so it cannot be nested inside another
                    one and cannot be wrapped in a <label> without either doing
                    nothing or toggling twice. And it takes `onCheckedChange`,
                    not `onChange` - which is the actual bug: the box was handed
                    a callback it has never read, so clicking it did nothing at
                    all.
                  */
                  <div
                    key={agent.id}
                    role="checkbox"
                    tabIndex={0}
                    aria-checked={picked.has(agent.id)}
                    aria-label={`Assign ${agent.name} to ${computer.name}`}
                    onClick={() => toggle(agent.id)}
                    onKeyDown={(event) => {
                      if (event.key === " " || event.key === "Enter") {
                        event.preventDefault();
                        toggle(agent.id);
                      }
                    }}
                    className="flex w-full cursor-pointer items-center gap-3 rounded-xl fill-whisper px-3 py-2 text-left outline-none focus-visible:fill-control-hover"
                  >
                    <Checkbox checked={picked.has(agent.id)} tabIndex={-1} aria-hidden="true" />
                    <Avatar name={agent.name} src={agent.avatarUrl} size="sm" />
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-[13px] text-foreground">{agent.name}</p>
                      <p className="truncate text-[11px] text-muted-foreground">
                        {elsewhere && !picked.has(agent.id)
                          ? "On another computer"
                          : elsewhere
                            ? "Will move here from another computer"
                            : (agent.role ?? "").slice(0, 80) || "No role set"}
                      </p>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </ScrollArea>
      </DialogBody>

      <DialogFooter>
        <Button size="pill" variant="secondary" onClick={() => onOpenChange(false)}>
          Cancel
        </Button>
        <Button size="pill" onClick={save} disabled={busy}>
          {busy ? "Saving…" : "Save"}
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
