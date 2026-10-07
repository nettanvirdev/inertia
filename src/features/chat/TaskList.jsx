import * as React from "react";
import { Circle, CircleCheck, CircleX } from "@/components/icons";
import { cn } from "@/lib/utils";

/**
 * The agent's task list, as rows.
 *
 * One component, two places: the todowrite card in the transcript, where it is
 * the record of a plan being written, and the plan popover in the header, where
 * it is the plan as it stands right now. They are the same list and there is no
 * version of this where they should look different.
 */

export const TODO_STATE = {
  completed: { label: "done", icon: CircleCheck, ink: "text-success-ink" },
  in_progress: { label: "in progress", icon: Circle, ink: "text-foreground" },
  cancelled: { label: "cancelled", icon: CircleX, ink: "text-muted-foreground" },
  pending: { label: "to do", icon: Circle, ink: "text-muted-foreground" },
};

/** Items still to do - what "2 tasks left" counts, and what decides "finished". */
export function openTodos(todos) {
  return (todos ?? []).filter((todo) => todo.status !== "completed" && todo.status !== "cancelled");
}

export function TaskList({ todos, className }) {
  return (
    <ul className={cn("flex flex-col gap-1", className)}>
      {todos.map((todo, i) => {
        const state = TODO_STATE[todo.status] ?? TODO_STATE.pending;
        const Glyph = state.icon;
        const struck = todo.status === "completed" || todo.status === "cancelled";
        return (
          <li key={todo.id ?? i} className="flex items-start gap-2 text-[13px]">
            <Glyph className={cn("mt-0.5 size-3.5 shrink-0", state.ink)} aria-hidden="true" />
            <span
              className={cn(
                "min-w-0 flex-1",
                struck ? "text-muted-foreground line-through" : "text-foreground"
              )}
            >
              {todo.content ?? todo.title ?? String(todo)}
            </span>
            <span className="shrink-0 text-[11px] text-muted-foreground">{state.label}</span>
          </li>
        );
      })}
    </ul>
  );
}
