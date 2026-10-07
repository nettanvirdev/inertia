import * as React from "react";
import { Search, X } from "@/components/icons";
import { cn } from "@/lib/utils";

// The fill is what makes it read as an input, and it is the only thing that
// does: there is no edge on any control in this app any more. So the fill has
// to be a real one - #e9edf3 on white, #313133 on #181818 - with its own step
// for hover and another for focus, because a field with no outline still has
// to answer the cursor and the tab key.
//
// The resting fill is the one hover used to use. A field that only becomes
// visible once the pointer is on it is a field you cannot find, and finding it
// is the state that matters.
const SIZES = {
  xs: "h-7 gap-1.5 rounded-lg px-2.5 text-xs",
  sm: "h-8 gap-2 rounded-md px-3 text-[13px]",
  md: "h-9 gap-2 rounded-full px-3.5 text-sm",
};

const Input = React.forwardRef(function Input(
  { className, size = "md", leadingIcon, trailingSlot, disabled, ...props },
  ref
) {
  return (
    <div
      className={cn(
        "relative flex w-full items-center bg-input text-input-foreground",
        "transition-colors duration-150 ease-out hover:bg-input-hover",
        // Focus is a further step of the same fill rather than a ring. A
        // 2px outline on a focused field was the loudest mark on the screen
        // and it appeared under the mouse as well as under the tab key.
        "focus-within:bg-input-focus",
        "[&_svg]:shrink-0",
        size === "xs" ? "[&_svg]:size-3.5" : "[&_svg]:size-4",
        disabled && "pointer-events-none opacity-50",
        SIZES[size] ?? SIZES.md,
        className
      )}
    >
      {leadingIcon ? (
        <span aria-hidden="true" className="flex shrink-0 items-center text-muted-foreground">
          {leadingIcon}
        </span>
      ) : null}
      <input
        ref={ref}
        disabled={disabled}
        className={cn(
          "h-full w-full min-w-0 flex-1 bg-transparent outline-none",
          "placeholder:text-input-placeholder",
          "selection:bg-foreground selection:text-background"
        )}
        {...props}
      />
      {trailingSlot ? <span className="flex shrink-0 items-center">{trailingSlot}</span> : null}
    </div>
  );
});

const SearchInput = React.forwardRef(function SearchInput(
  { className, size = "md", value, onClear, placeholder = "Search", ...props },
  ref
) {
  const hasValue = value != null && String(value).length > 0;

  return (
    <Input
      ref={ref}
      size={size}
      // not type="search": the engine paints its own native clear affordance
      type="text"
      role="searchbox"
      value={value}
      placeholder={placeholder}
      className={className}
      leadingIcon={<Search />}
      trailingSlot={
        onClear && hasValue ? (
          <button
            type="button"
            aria-label="Clear search"
            onClick={onClear}
            className={cn(
              "flex size-5 items-center justify-center rounded-full text-muted-foreground",
              "outline-none transition-colors duration-150 ease-out",
              "hover:fill-close hover:text-foreground",
              "focus-visible:fill-close focus-visible:text-foreground"
            )}
          >
            <X className="size-3" />
          </button>
        ) : null
      }
      {...props}
    />
  );
});

export { Input, SearchInput };
