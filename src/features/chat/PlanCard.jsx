import * as React from "react";
import { CircleAlert, ListChecks, MessageCircle, Zap } from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { latestPlan } from "@shared/transcript";

/**
 * A plan, with the decision attached to it.
 *
 * This is the hinge of the whole Chat → Plan → Autonomous flow. A plan written
 * as prose leaves approval to the user typing "yes", which means the thing being
 * agreed to is whatever the model remembers of what it said. Rendered from the
 * `present_plan` call's own arguments, the steps on screen are literally the
 * ones recorded, and pressing Build sends them back as the brief.
 *
 * ## Only the newest plan asks
 *
 * A conversation can go round twice - plan, discuss, plan again - and every one
 * of those cards stays in the transcript. Buttons on all of them would be three
 * ways to start three different builds. So the buttons appear on the last plan
 * only, and only while the conversation is still in a mode that has not started
 * building.
 */
export function PlanCard({ plan, callId }) {
  const { activeThreadId, threads, messages, setThreadMode, sendMessage } = useApp();
  const [dismissed, setDismissed] = React.useState(false);

  const thread = threads.find((t) => t.id === activeThreadId);
  const newest = React.useMemo(
    () => latestPlan(messages[activeThreadId] ?? []),
    [messages, activeThreadId]
  );

  // Approving switches the conversation to Autonomous, so "not autonomous yet"
  // is the same question as "has this been approved" without storing a flag that
  // could disagree with the mode the agent is actually running in.
  const open =
    !dismissed &&
    thread?.mode !== "autonomous" &&
    (!newest || !callId || newest.callId === callId);

  function build() {
    if (!activeThreadId) return;
    setThreadMode(activeThreadId, "autonomous");
    // The plan goes back as the instruction rather than a bare "yes", so the
    // building turn is working from the approved text and not from its own
    // recollection of it several thousand tokens ago.
    sendMessage(
      activeThreadId,
      [
        `Approved: ${plan.title}. Build it.`,
        "",
        "The plan, as approved:",
        ...plan.steps.map((step, i) => `${i + 1}. ${step.title}${step.detail ? ` - ${step.detail}` : ""}`),
        "",
        "Work through it in order and keep the task list in step with where you are.",
      ].join("\n"),
      { mode: "autonomous" }
    );
  }

  return (
    <div className="flex flex-col gap-3 rounded-2xl fill-whisper p-3.5">
      <div className="flex items-start gap-2">
        <ListChecks className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p className="text-[13px] font-medium text-foreground">{plan.title}</p>
          {plan.summary ? (
            <p className="mt-0.5 text-[12px] leading-relaxed text-muted-foreground">{plan.summary}</p>
          ) : null}
        </div>
      </div>

      <ol className="flex flex-col gap-1.5">
        {plan.steps.map((step, i) => (
          <li key={`${i}-${step.title}`} className="flex gap-2.5">
            <span
              aria-hidden="true"
              className="mt-px grid size-[1.125rem] shrink-0 place-items-center rounded-full fill-control text-[10px] tabular-nums text-muted-foreground"
            >
              {i + 1}
            </span>
            <span className="min-w-0 flex-1">
              <span className="block text-[12.5px] leading-snug text-foreground">{step.title}</span>
              {step.detail ? (
                <span className="mt-0.5 block text-[11px] leading-snug text-muted-foreground">
                  {step.detail}
                </span>
              ) : null}
            </span>
          </li>
        ))}
      </ol>

      {plan.risks?.length ? (
        <div className="flex flex-col gap-1 rounded-xl bg-warning-wash px-3 py-2">
          {plan.risks.map((risk, i) => (
            <p
              key={i}
              className="flex items-start gap-1.5 text-[11px] leading-relaxed text-warning-ink"
            >
              <CircleAlert className="mt-px size-3.5 shrink-0" aria-hidden="true" />
              <span>{risk}</span>
            </p>
          ))}
        </div>
      ) : null}

      {/* Two folds rather than a swap: the buttons fold away as the line that
          replaces them folds open, so the card settles to its new height
          instead of jumping to it. Neither moves on first render - an old plan
          simply shows whichever it is at. */}
      <Collapse open={open} innerClassName="flex flex-wrap items-center gap-1.5">
        <Button size="xs" onClick={build}>
          <Zap className="size-3.5" aria-hidden="true" />
          Build it
        </Button>
        <Button size="xs" variant="subtle" onClick={() => setDismissed(true)}>
          <MessageCircle className="size-3.5" aria-hidden="true" />
          Discuss the plan
        </Button>
        <span className="pl-1 text-[11px] text-muted-foreground">
          Building switches this chat to Autonomous.
        </span>
      </Collapse>
      <Collapse open={!open}>
        <p className={cn("text-[11px] text-muted-foreground")}>
          {thread?.mode === "autonomous" ? "Approved - building." : "Say what you would change."}
        </p>
      </Collapse>
    </div>
  );
}
