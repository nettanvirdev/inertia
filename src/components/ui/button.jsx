import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva } from "class-variance-authority";
import { cn } from "@/lib/utils";

/**
 * The loud button in this system is neutral-inverted unless the reader has
 * chosen an accent, in which case it is the one place a colour is allowed to be
 * solid.
 *
 * Every button is a pill now, at 36px and 30px, which is what app-controls
 * specifies for both of its sizes.
 *
 * There are two reds because the spec and this app disagree, and both are
 * right about different buttons. `destructive` is the spec's: a solid red slab
 * with its own hover and pressed tints, for the confirm button in a dialog
 * that is about to delete something. `danger` is red ink on a red wash, for a
 * destructive action sitting inline in a list - twenty solid red slabs down a
 * settings page is a screen that is shouting, and the one that actually
 * matters no longer stands out.
 */
const buttonVariants = cva(
  [
    "inline-flex shrink-0 items-center justify-center gap-2 whitespace-nowrap",
    "font-normal outline-none transition-colors duration-150 ease-out",
    "disabled:pointer-events-none disabled:opacity-40",
    // No focus ring anywhere in the app, so focus is said by the same move
    // each variant already makes on hover. It cannot live here in the base:
    // a fill applied to `primary` would paint over the solid accent and turn
    // the loudest button on the screen into a grey one the moment it is
    // tabbed to.
    "[&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
  ],
  {
    variants: {
      variant: {
        // hover is opacity, not a fill - there is no darker shade of "inverted"
        // accent-fill IS bg-foreground/text-background until an accent exists,
        // so the neutral-inverted default below is unchanged by this
        primary: "accent-fill hover:opacity-80 focus-visible:opacity-80 active:opacity-90",
        // A fill, where it used to be a hairline around nothing.
        //
        // The old version was transparent with an outline, which took whatever
        // surface it sat on and let the edge do the work. With no edges left
        // in the app that leaves a button-shaped hole, so this is now the
        // secondary fill - one rung firmer than `subtle`, which is what tells
        // the two apart now that neither is outlined.
        secondary: [
          "fill-secondary text-foreground",
          "hover:fill-secondary-hover focus-visible:fill-secondary-hover",
          "active:bg-control-pressed",
        ].join(" "),
        ghost: "bg-transparent text-foreground hover:fill-nav focus-visible:fill-nav",
        subtle:
          "fill-control text-foreground hover:fill-control-hover focus-visible:fill-control-hover",
        destructive:
          "bg-destructive text-destructive-foreground hover:bg-destructive-hover focus-visible:bg-destructive-hover active:bg-destructive-pressed",
        // The same idea for the dangerous one: a wash of the destructive
        // colour instead of a ring of it. Kept distinct from `danger` by
        // carrying the full-strength label rather than the ink one.
        destructiveOutline: [
          "bg-destructive/10 text-destructive",
          "hover:bg-destructive/15 hover:text-destructive-hover",
          "focus-visible:bg-destructive/15 focus-visible:text-destructive-hover",
          "active:bg-destructive/20 active:text-destructive-pressed",
        ].join(" "),
        danger:
          "bg-destructive/10 text-destructive-ink hover:bg-destructive/15 focus-visible:bg-destructive/15 dark:text-destructive-ink",
      },
      // 36px and 30px are the spec's two sizes; xs and lg are this app's, kept
      // because a toolbar and a dialog footer are not the same problem.
      size: {
        xs: "h-7 rounded-full px-3 text-xs",
        sm: "h-[1.875rem] rounded-full px-3.5 text-[13px]",
        md: "h-9 rounded-full px-4 text-sm",
        lg: "h-10 rounded-full px-6 text-sm",
        // Kept as its own name because call sites use it; same shape as `sm`
        // now that every button is a pill, one step wider.
        pill: "h-8 rounded-full px-4 text-[13px]",
      },
    },
    defaultVariants: { variant: "secondary", size: "md" },
  }
);

const Button = React.forwardRef(function Button(
  { className, variant, size, asChild = false, type, ...props },
  ref
) {
  const Comp = asChild ? Slot : "button";
  return (
    <Comp
      ref={ref}
      type={asChild ? type : (type ?? "button")}
      className={cn(buttonVariants({ variant, size }), className)}
      {...props}
    />
  );
});

export { Button, buttonVariants };
