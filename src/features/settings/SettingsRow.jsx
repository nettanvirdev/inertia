import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * The three building blocks every settings pane is made of.
 *
 * They exist so the panes stay declarative and, more importantly, so ONE file
 * decides the settings density band: 28px controls, 12px labels, 11px
 * descriptions. A 36px control never sneaks in because no pane hand-rolls a row.
 */

export function SettingsSection({
  title,
  description,
  children,
  flat = false,
  className,
  contentClassName,
  // For a section whose group has to be measured or scrolled - a long list
  // capped at a few rows. The group is built here, so a pane that needs to
  // reach it, or to give it a height it had to measure first, has no other
  // way in. `contentStyle` rather than a class because the number is computed
  // at runtime and Tailwind cannot name it.
  contentRef,
  contentStyle,
  ...props
}) {
  return (
    <section className={cn("mt-7 first:mt-0", className)} {...props}>
      {title ? (
        // A heading, not a label. This was 11px uppercase muted - the size and
        // colour of a caption - so a pane read as one undifferentiated column
        // with faint captions floating in it. At 13px semibold in the
        // foreground it is the thing you scan down the page to find.
        <h3 className="text-[0.8125rem] font-semibold text-foreground">
          {title}
        </h3>
      ) : null}
      {description ? (
        <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
          {description}
        </p>
      ) : null}
      {/* contentClassName merges last so a pane can swap the group for a
          multi-column flow without hand-rolling the section markup */}
      <div
        ref={contentRef}
        style={contentStyle}
        className={cn(
          flat
            ? "flex flex-col gap-2.5"
            : "divide-border-subtle card-surface divide-y overflow-hidden rounded-2xl",
          (title || description) && "mt-2.5",
          contentClassName,
        )}
      >
        {children}
      </div>
    </section>
  );
}

/**
 * Label block left, control right. Below `sm` the control drops under the label
 * - at 28px a control squeezed beside a wrapped two-line label reads as broken.
 *
 * The text block has no measure cap of its own. It used to, and the result was a
 * pane that wrapped every description at roughly half the available width while
 * a lone control sat far off to the right - a row that looked broken on any
 * window wider than a laptop. The line length is bounded by two real things
 * instead: the control it has to stop short of, and the column cap the dialog
 * puts on the pane.
 */
export function SettingsRow({
  label,
  description,
  control,
  htmlFor,
  className,
  children,
  ...props
}) {
  const Label = htmlFor ? "label" : "div";
  return (
    <div
      className={cn(
        // The padding lives on the row rather than the group, so the hairline
        // between two rows runs the full width of the card instead of stopping
        // short of it - which is the difference between a list and a stack of
        // things that happen to be near each other.
        "flex flex-col gap-1.5 px-4 py-3 sm:flex-row sm:items-center sm:justify-between sm:gap-8",
        className,
      )}
      {...props}
    >
      <div className="min-w-0 flex-1">
        {label ? (
          <Label
            htmlFor={htmlFor}
            className={cn(
              "block text-[0.8125rem] text-foreground",
              htmlFor && "cursor-pointer select-none",
            )}
          >
            {label}
          </Label>
        ) : null}
        {description ? (
          <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
            {description}
          </p>
        ) : null}
      </div>
      {(control ?? children) ? (
        <div className="flex shrink-0 items-center gap-1.5 sm:justify-end">
          {control ?? children}
        </div>
      ) : null}
    </div>
  );
}

/**
 * A flat grouping surface - one step up the ladder, never a bordered box.
 *
 * It carries its own padding. It did not, and every caller that did not think
 * to add some got a block of text welded to the card's own edge - which is what
 * a card is for avoiding. A caller that wants a tighter inset (a list of rows
 * that pad themselves) still overrides it.
 */
export function SettingsCard({ children, className, ...props }) {
  return (
    <div className={cn("rounded-lg card-surface-subtle p-3", className)} {...props}>
      {children}
    </div>
  );
}

/** A field: label above, full-width control below. Used where a row would crowd. */
export function SettingsField({
  label,
  description,
  htmlFor,
  children,
  className,
  ...props
}) {
  return (
    <div className={cn("min-w-0", className)} {...props}>
      {label ? (
        <label htmlFor={htmlFor} className="block text-xs text-foreground/90">
          {label}
        </label>
      ) : null}
      <div className="mt-1">{children}</div>
      {description ? (
        <p className="mt-1 text-[0.6875rem] leading-relaxed text-muted-foreground">
          {description}
        </p>
      ) : null}
    </div>
  );
}
