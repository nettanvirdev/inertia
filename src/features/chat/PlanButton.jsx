import * as React from "react";
import { ListTodo } from "@/components/icons";
import { cn } from "@/lib/utils";
import { IconButton } from "@/components/ui/icon-button";
import { Popover } from "@/components/ui/popover";
import { TaskList, openTodos } from "@/features/chat/TaskList";

/**
 * The plan, in the header.
 *
 * The task list is written into the transcript and then buried by everything
 * that happens after it, which is exactly when it is most wanted: twelve tool
 * calls later, "what is it still going to do?" means scrolling back to a card
 * that has moved a long way up. This puts the current answer one glance away
 * and keeps it there, and it reads from the same tool part the card does, so
 * the two cannot disagree and it updates as the turn writes.
 *
 * Hover opens it, because a plan is something you check rather than something
 * you operate; a click holds it open for a pointer that is going somewhere
 * else and for anyone arriving by keyboard. Leaving has a short grace period,
 * or crossing the gap between the button and the sheet would close it.
 */

const OPEN_DELAY = 120;
const CLOSE_GRACE = 180;

export function PlanButton({ todos }) {
  const anchorRef = React.useRef(null);
  const timer = React.useRef(null);
  const [open, setOpen] = React.useState(false);
  // A click pins the sheet: leaving with the pointer no longer closes it.
  const [held, setHeld] = React.useState(false);

  const clear = () => {
    clearTimeout(timer.current);
    timer.current = null;
  };
  React.useEffect(() => clear, []);

  const show = () => {
    clear();
    timer.current = setTimeout(() => setOpen(true), OPEN_DELAY);
  };

  const hide = () => {
    if (held) return;
    clear();
    timer.current = setTimeout(() => setOpen(false), CLOSE_GRACE);
  };

  const close = () => {
    clear();
    setHeld(false);
    setOpen(false);
  };

  const left = openTodos(todos);
  const done = todos.filter((todo) => todo.status === "completed").length;
  const label = left.length
    ? `Plan: ${left.length} of ${todos.length} left`
    : `Plan: all ${todos.length} done`;

  return (
    <>
      <IconButton
        ref={anchorRef}
        size="lg"
        label={label}
        data-state={open ? "open" : "closed"}
        onPointerEnter={show}
        onPointerLeave={hide}
        onFocus={() => setOpen(true)}
        onClick={() => {
          setHeld((v) => !v);
          setOpen(true);
        }}
      >
        <span className="relative">
          <ListTodo />
          {/* A dot rather than a number: the count is in the sheet a moment
              later, and what the header has to say from across the room is
              only that there is something left. */}
          {left.length ? (
            <span
              aria-hidden="true"
              className="absolute -right-0.5 -top-0.5 size-1.5 rounded-full bg-foreground"
            />
          ) : null}
        </span>
      </IconButton>

      <Popover
        open={open}
        onOpenChange={close}
        anchorRef={anchorRef}
        side="bottom"
        align="end"
        offset={6}
        ariaLabel="Plan"
        className={cn("w-[min(22rem,calc(100vw-1rem))] p-3")}
        onPointerEnter={clear}
        onPointerLeave={hide}
      >
        <div className="mb-2 flex items-baseline justify-between gap-2">
          <span className="text-[13px] font-medium text-foreground">Plan</span>
          <span className="text-[11px] tabular-nums text-muted-foreground">
            {`${done}/${todos.length} done`}
          </span>
        </div>
        <TaskList todos={todos} />
      </Popover>
    </>
  );
}
