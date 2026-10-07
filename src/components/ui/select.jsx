import * as React from "react";
import { Check, ChevronDown } from "@/components/icons";
import { cn } from "@/lib/utils";
import { Popover } from "@/components/ui/popover";
import { menuRowClass } from "@/components/ui/dropdown-menu";

/**
 * Replaces the native <select> outright. The trigger is a control-fill row that
 * matches the input beside it; the sheet is the dropdown sheet.
 *
 * `options` are flat - `{ value, label, icon, description, disabled }` - or
 * grouped - `{ group: "Label", options: [...] }`. Both may be mixed.
 */

const triggerSizes = {
  xs: "h-7 rounded-lg px-2.5 text-xs gap-1.5",
  md: "h-9 rounded-lg px-3 text-[13px] gap-2",
};

function flatten(options = []) {
  const flat = [];
  for (const entry of options) {
    if (entry && Array.isArray(entry.options)) {
      flat.push({ __group: entry.group ?? entry.label });
      for (const opt of entry.options) flat.push(opt);
    } else if (entry) {
      flat.push(entry);
    }
  }
  return flat;
}

function renderIcon(icon, className) {
  if (!icon) return null;
  if (React.isValidElement(icon)) {
    return React.cloneElement(icon, { className: cn(className, icon.props.className) });
  }
  const Icon = icon;
  return <Icon className={className} />;
}

export function Select({
  value,
  onChange,
  options = [],
  placeholder = "Select…",
  /** What the panel says when there is nothing to choose. Callers that know
   *  why the list is empty should say so - "No agents yet" beats "Nothing". */
  emptyLabel = "Nothing to choose from yet",
  size = "md",
  disabled,
  className,
  panelClassName,
  renderValue,
  ariaLabel,
  id,
}) {
  const triggerRef = React.useRef(null);
  const listRef = React.useRef(null);
  const [open, setOpen] = React.useState(false);
  const baseId = React.useId();

  const rows = React.useMemo(() => flatten(options), [options]);
  const selectable = React.useMemo(() => rows.filter((r) => !r.__group && !r.disabled), [rows]);
  const selected = React.useMemo(
    () => rows.find((r) => !r.__group && r.value === value) ?? null,
    [rows, value]
  );

  // "Select…" invites a click that opens nothing. When there is nothing to
  // choose, the trigger says that instead and does not pretend to be actionable.
  const isEmpty = rows.length === 0;

  const [activeValue, setActiveValue] = React.useState(value);
  const activeIndex = selectable.findIndex((o) => o.value === activeValue);
  const optionId = (v) => `${baseId}-opt-${String(v)}`;

  // opening always starts from the current selection
  React.useEffect(() => {
    if (open) setActiveValue(value ?? selectable[0]?.value);
  }, [open, value]); // eslint-disable-line react-hooks/exhaustive-deps

  // return focus to the trigger when the sheet goes away
  const wasOpen = React.useRef(open);
  React.useEffect(() => {
    if (wasOpen.current && !open) triggerRef.current?.focus({ preventScroll: true });
    wasOpen.current = open;
  }, [open]);

  React.useEffect(() => {
    if (!open || activeValue === undefined) return;
    const el = listRef.current?.querySelector(`#${CSS.escape(optionId(activeValue))}`);
    el?.scrollIntoView({ block: "nearest" });
  }, [open, activeValue]); // eslint-disable-line react-hooks/exhaustive-deps

  const commit = (option) => {
    if (!option || option.disabled) return;
    onChange?.(option.value, option);
    setOpen(false);
  };

  const move = (delta) => {
    if (!selectable.length) return;
    const from = activeIndex < 0 ? (delta > 0 ? -1 : 0) : activeIndex;
    const next = (from + delta + selectable.length) % selectable.length;
    setActiveValue(selectable[next].value);
  };

  const onTriggerKeyDown = (e) => {
    if (disabled) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      setOpen(true);
    }
  };

  const onListKeyDown = (e) => {
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        move(1);
        break;
      case "ArrowUp":
        e.preventDefault();
        move(-1);
        break;
      case "Home":
        e.preventDefault();
        setActiveValue(selectable[0]?.value);
        break;
      case "End":
        e.preventDefault();
        setActiveValue(selectable[selectable.length - 1]?.value);
        break;
      case "Enter":
      case " ":
        e.preventDefault();
        commit(selectable[activeIndex]);
        break;
      case "Tab":
        setOpen(false);
        break;
      default:
        break;
    }
  };

  React.useEffect(() => {
    if (!open) return;
    const raf = requestAnimationFrame(() => listRef.current?.focus({ preventScroll: true }));
    return () => cancelAnimationFrame(raf);
  }, [open]);

  return (
    <>
      <button
        ref={triggerRef}
        id={id}
        type="button"
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        aria-controls={open ? `${baseId}-list` : undefined}
        disabled={disabled}
        data-state={open ? "open" : "closed"}
        onClick={() => setOpen((v) => !v)}
        onKeyDown={onTriggerKeyDown}
        className={cn(
          "inline-flex w-full items-center justify-between font-normal text-foreground outline-none",
          "fill-control hover:fill-control-hover transition-colors duration-150 ease-out",
          // Open is the same weight as hover, and the same fill every other
          // picker trigger uses when it is open. It was `bg-muted`, which is a
          // different colour from the hover it replaces - so a select opened
          // by keyboard changed shade, and a select sitting beside a dropdown
          // pill was visibly not the same control.
          "data-[state=open]:fill-control-hover",
          "focus-visible:fill-control-hover",
          "disabled:pointer-events-none disabled:opacity-50",
          triggerSizes[size] ?? triggerSizes.md,
          className
        )}
      >
        <span className="flex min-w-0 flex-1 items-center gap-2 truncate text-left">
          {renderValue ? (
            renderValue(selected)
          ) : selected ? (
            <>
              {renderIcon(selected.icon, "size-3.5 shrink-0 text-muted-foreground")}
              <span className="truncate">{selected.label}</span>
            </>
          ) : (
            <span className="truncate text-muted-foreground">
              {isEmpty ? emptyLabel : placeholder}
            </span>
          )}
        </span>
        <ChevronDown
          className={cn(
            "size-3.5 shrink-0 text-muted-foreground transition-transform duration-150 ease-out",
            open && "rotate-180"
          )}
        />
      </button>

      <Popover
        open={open}
        onOpenChange={setOpen}
        anchorRef={triggerRef}
        side="bottom"
        align="start"
        offset={6}
        matchWidth="anchor"
        className={cn(
          // The width of the trigger, so the sheet reads as the control opening
          // rather than as a panel landing beside it. A label longer than that
          // truncates in the row, exactly as it does in the trigger.
          "max-w-[calc(100vw-1rem)]",
          panelClassName
        )}
        role="presentation"
      >
        <div
          ref={listRef}
          id={`${baseId}-list`}
          role="listbox"
          tabIndex={-1}
          aria-activedescendant={activeValue !== undefined ? optionId(activeValue) : undefined}
          onKeyDown={onListKeyDown}
          className="outline-none"
        >
          {/*
            A select with nothing in it used to open a panel with nothing in it -
            a small blank rectangle, no explanation, and no way to tell it apart
            from a control that had failed to load. It happens for real: the
            default-agent picker in a workspace with no agents, the
            default-computer picker before a machine has been made. Saying so is
            one line and removes a whole class of "is this broken?".
          */}
          {rows.length === 0 ? (
            <div role="presentation" className="px-2.5 py-2 text-[12px] text-muted-foreground">
              {emptyLabel}
            </div>
          ) : null}
          {rows.map((row, i) =>
            row.__group ? (
              <div
                key={`g-${i}`}
                role="presentation"
                className="px-2.5 pb-1 pt-1.5 text-[11px] font-medium text-muted-foreground"
              >
                {row.__group}
              </div>
            ) : (
              <div
                key={String(row.value)}
                id={optionId(row.value)}
                role="option"
                aria-selected={row.value === value}
                aria-disabled={row.disabled || undefined}
                data-disabled={row.disabled ? "" : undefined}
                data-active={row.value === activeValue ? "true" : undefined}
                onClick={() => commit(row)}
                onPointerEnter={() => !row.disabled && setActiveValue(row.value)}
                className={cn(
                  menuRowClass,
                  row.description && "h-auto py-1.5",
                  "cursor-pointer data-[active=true]:fill-menu",
                  "data-[active=true]:[&_svg:not([class*='text-'])]:text-foreground"
                )}
              >
                {renderIcon(row.icon, "size-3.5")}
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate">{row.label}</span>
                  {row.description ? (
                    <span className="truncate text-[11px] text-muted-foreground">
                      {row.description}
                    </span>
                  ) : null}
                </span>
                <span className="flex w-3.5 shrink-0 justify-end">
                  {row.value === value ? <Check className="size-3.5 text-foreground" /> : null}
                </span>
              </div>
            )
          )}
        </div>
      </Popover>
    </>
  );
}
