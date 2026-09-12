import * as React from "react";
import { MessageCircleQuestion } from "@/components/icons";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { RadioGroup, RadioItem } from "@/components/ui/radio";
import { Textarea } from "@/components/ui/textarea";

/**
 * The agent asking the user something.
 *
 * Same reasoning as the permission card: this lives in the transcript rather
 * than over it, because the question is nearly always about work that is
 * visible a few lines up, and the answer is better when the reader can look.
 *
 * The free-text box is not a fallback for a missing option - it is always
 * there. A list of options is the model's guess at the shape of the answer,
 * and the times that guess is wrong are exactly the times the answer matters.
 */

export function QuestionPrompt({ question, onAnswer, onDismiss, className }) {
  const [choice, setChoice] = React.useState(null);
  const [text, setText] = React.useState("");
  const options = question?.options ?? [];

  const answer = React.useCallback(
    (value) => {
      const reply = value ?? (text.trim() || choice);
      if (!reply) return;
      onAnswer?.(question.id, reply);
    },
    [choice, onAnswer, question?.id, text]
  );

  function handleKeyDown(event) {
    if (event.key === "Escape") {
      event.preventDefault();
      onDismiss?.(question.id);
      return;
    }
    if (event.key !== "Enter" || event.shiftKey) return;
    // The radio rows are buttons and handle Enter as "pick me"; letting this
    // fire too would pick an option and submit it in the same keystroke.
    if (event.target instanceof HTMLButtonElement) return;
    event.preventDefault();
    answer();
  }

  if (!question) return null;

  const ready = Boolean(text.trim() || choice);

  return (
    <div
      role="group"
      aria-label="Question from the agent"
      onKeyDown={handleKeyDown}
      /* No outline, for the reason `PermissionPrompt` gives: the transcript
         separates everything in it by a step of colour, and the one card with
         a box around it reads as a dialog that failed to open. */
      className={cn("my-3 flex flex-col gap-3 rounded-2xl card-surface-subtle p-4", className)}
    >
      <div className="flex items-start gap-2.5">
        <MessageCircleQuestion
          aria-hidden="true"
          className="mt-0.5 size-4 shrink-0 text-muted-foreground"
        />
        <h3 className="min-w-0 flex-1 text-[13px] leading-relaxed font-medium text-foreground">
          {question.question}
        </h3>
      </div>

      {options.length ? (
        <RadioGroup
          label={question.question}
          value={choice}
          onChange={(value) => {
            setChoice(value);
            // Choosing clears a half-typed answer rather than leaving two
            // competing ones on screen with no sign of which will be sent.
            setText("");
          }}
        >
          {options.map((option) => (
            <RadioItem
              key={option.label}
              value={option.label}
              label={option.label}
              description={option.description}
            />
          ))}
        </RadioGroup>
      ) : null}

      <div className="flex flex-col gap-2">
        <label htmlFor={`${question.id}-other`} className="text-[11px] text-muted-foreground">
          {options.length ? "Or answer in your own words" : "Your answer"}
        </label>
        <Textarea
          id={`${question.id}-other`}
          rows={2}
          autoResize
          maxRows={8}
          value={text}
          onChange={(event) => {
            setText(event.target.value);
            if (event.target.value.trim()) setChoice(null);
          }}
          placeholder="Type an answer"
        />
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button variant="primary" size="sm" disabled={!ready} onClick={() => answer()}>
          Answer
          <Kbd className="bg-background/20 text-background/80">↵</Kbd>
        </Button>
        <Button variant="ghost" size="sm" onClick={() => onDismiss?.(question.id)}>
          Dismiss
          <Kbd className="bg-transparent opacity-70">Esc</Kbd>
        </Button>
      </div>
    </div>
  );
}
