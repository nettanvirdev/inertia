import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva } from "class-variance-authority";
import { cn } from "@/lib/utils";
import { InsideTooltip, Tooltip } from "@/components/ui/tooltip";

/**
 * The icon's own two rungs, rather than the text rungs.
 *
 * An icon at rest was `text-muted-foreground`, which is the colour of a caption
 * - and a 16px glyph at caption weight is fainter than a caption, because it
 * has none of the mass a word has. The palette names `icon.muted` and
 * `icon.default` for exactly this, one step firmer than the text pair, and
 * hover moves between them.
 */
const iconButtonVariants = cva(
  [
    "inline-flex shrink-0 items-center justify-center rounded-full",
    "text-icon-muted outline-none",
    "transition-colors duration-150 ease-out",
    "hover:fill-nav hover:text-icon",
    // Focus is per variant for the same reason it is on `Button`: a fill on
    // top of the prominent one would rub out the fill that makes it prominent.
    "disabled:pointer-events-none disabled:opacity-40",
    // a control that opened a floating layer stays visibly held
    "data-[state=open]:bg-muted data-[state=open]:text-icon",
    "[&_svg]:pointer-events-none [&_svg]:shrink-0",
  ],
  {
    variants: {
      size: {
        sm: "size-6 [&_svg]:size-3.5",
        md: "size-[1.875rem] [&_svg]:size-4",
        lg: "size-8 [&_svg]:size-4",
      },
      variant: {
        // appears from nothing on hover - the default target
        ghost: "focus-visible:fill-nav focus-visible:text-icon",
        // Emphasis, not inversion.
        //
        // This was the palette's inverted chip - a solid mid-grey circle with
        // its contrast the other way round - and on screen it read as a blob:
        // heavier than the primary button next to it, and the same grey in
        // both themes, so neither mode looked like it had been designed. A
        // step up the surface ladder says "this one matters" without shouting,
        // and it is the same gesture in both modes.
        prominent: "bg-muted text-foreground hover:bg-border-strong focus-visible:bg-border-strong",
      },
      active: {
        true: "bg-muted text-icon focus-visible:bg-border-strong",
        false: "",
      },
    },
    defaultVariants: { size: "md", variant: "ghost", active: false },
  }
);

/**
 * An icon with no words, and the words on hover.
 *
 * Every one of these already carried a `label`, and every one of them put it
 * in `aria-label` and nowhere else - so a screen reader knew what the button
 * did and a person looking at it did not. Eighty-odd buttons across the app,
 * each a small square with a glyph in it, and the only way to find out what
 * one meant was to press it.
 *
 * So the label is the tooltip, everywhere, by default. `tooltip` turns it off
 * for a button whose meaning is already written beside it, and a call site
 * that has wrapped this in a `Tooltip` of its own is detected rather than
 * having to say so - see `InsideTooltip`.
 */
const IconButton = React.forwardRef(function IconButton(
  {
    className,
    size = "md",
    variant = "ghost",
    label,
    active = false,
    asChild = false,
    type,
    tooltip = true,
    tooltipSide = "top",
    ...props
  },
  ref
) {
  const Comp = asChild ? Slot : "button";
  const wrapped = React.useContext(InsideTooltip);

  const button = (
    <Comp
      ref={ref}
      type={asChild ? type : (type ?? "button")}
      aria-label={label}
      aria-pressed={active || undefined}
      data-active={active ? "true" : undefined}
      className={cn(iconButtonVariants({ size, variant, active }), className)}
      {...props}
    />
  );

  // `disabled` is deliberately not a reason to skip it: a button that cannot
  // be pressed is exactly the one whose purpose is worth explaining.
  if (!tooltip || !label || wrapped) return button;
  return (
    <Tooltip content={label} side={tooltipSide}>
      {button}
    </Tooltip>
  );
});

export { IconButton, iconButtonVariants };
