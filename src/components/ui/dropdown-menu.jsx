import * as React from "react";
import { Check, ChevronRight } from "@/components/icons";
import { cn } from "@/lib/utils";
import { Kbd } from "@/components/ui/kbd";
import { Popover } from "@/components/ui/popover";

/* ── shared item recipe ─────────────────────────────────────────────────────
 * The highlighted row is a NEUTRAL alpha fill and its icon goes
 * muted-foreground → foreground at the same moment. Nothing turns a brand
 * colour. The `:not([class*='text-'])` guard lets an icon opt out by carrying
 * its own ink (the danger row and the trailing check do exactly that).
 * ─────────────────────────────────────────────────────────────────────────── */
const itemBase = [
  // rounded-sm (12px) rather than xl (20px): on a 32px row a 20px radius is
  // within 4px of a pill, and a pill-shaped highlight is what made these menus
  // read as loose. 12px still reads soft against the sheet's own 16px corner.
  "flex h-8 w-full shrink-0 items-center gap-2.5 rounded-sm px-2.5",
  "text-[13px] font-normal text-foreground text-left outline-none",
  "transition-colors duration-150 ease-out",
  "hover:fill-menu focus:fill-menu focus-visible:fill-menu",
  "data-[disabled]:pointer-events-none data-[disabled]:opacity-40",
  "[&_svg]:pointer-events-none [&_svg]:shrink-0",
  "[&_svg:not([class*='size-'])]:size-3.5",
  "[&_svg:not([class*='text-'])]:text-muted-foreground",
  "hover:[&_svg:not([class*='text-'])]:text-foreground",
  "focus:[&_svg:not([class*='text-'])]:text-foreground",
];

const dangerInk = "text-destructive-ink hover:bg-destructive-wash focus:bg-destructive-wash";

/** Exported so the Select listbox rows are literally the same row. */
export const menuRowClass = itemBase;

function renderIcon(icon, className) {
  if (!icon) return null;
  if (React.isValidElement(icon)) {
    return React.cloneElement(icon, { className: cn(className, icon.props.className) });
  }
  const Icon = icon;
  return <Icon className={className} />;
}

/* ── menu context ─────────────────────────────────────────────────────────── */

const MenuContext = React.createContext(null);

/** `close()` dismisses the nearest menu; `closeAll()` the whole stack. */
export function useMenu() {
  return React.useContext(MenuContext);
}

/* ── roving keyboard navigation ───────────────────────────────────────────── */

function getItems(panel) {
  if (!panel) return [];
  return Array.from(panel.querySelectorAll('[role^="menuitem"]')).filter(
    (el) => !el.hasAttribute("data-disabled")
  );
}

function focusAt(items, index) {
  const el = items[(index + items.length) % items.length];
  el?.focus();
}

/**
 * The portalled menu sheet: role, focus handling and roving arrow navigation.
 * Shared by DropdownMenu, SubMenu and ContextMenu so the three cannot drift.
 */
export const MenuPanel = React.forwardRef(function MenuPanel(
  {
    open,
    onOpenChange,
    onCloseAll,
    anchorRef,
    side = "bottom",
    align = "start",
    offset = 8,
    matchWidth,
    className,
    children,
    ariaLabel,
    onKeyDown: onKeyDownProp,
    ...props
  },
  ref
) {
  const panelRef = React.useRef(null);

  const close = React.useCallback(() => onOpenChange?.(false), [onOpenChange]);
  const closeAll = React.useCallback(() => {
    onCloseAll?.();
    close();
  }, [onCloseAll, close]);

  const ctx = React.useMemo(() => ({ close, closeAll }), [close, closeAll]);

  // The panel takes focus, not the first row: opening with the mouse should not
  // pre-highlight anything. ArrowDown/Up then enter the list from either end.
  React.useEffect(() => {
    if (!open) return;
    const id = requestAnimationFrame(() => panelRef.current?.focus({ preventScroll: true }));
    return () => cancelAnimationFrame(id);
  }, [open]);

  const onKeyDown = (e) => {
    onKeyDownProp?.(e);
    if (e.defaultPrevented) return;
    const panel = panelRef.current;
    if (!panel || !panel.contains(e.target)) return; // ignore bubbles from nested portals
    const items = getItems(panel);
    if (!items.length) return;
    const index = items.indexOf(document.activeElement);

    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        focusAt(items, index < 0 ? 0 : index + 1);
        break;
      case "ArrowUp":
        e.preventDefault();
        focusAt(items, index < 0 ? items.length - 1 : index - 1);
        break;
      case "Home":
        e.preventDefault();
        focusAt(items, 0);
        break;
      case "End":
        e.preventDefault();
        focusAt(items, items.length - 1);
        break;
      case "Tab":
        // Tab is an exit, not a traversal.
        closeAll();
        break;
      default:
        break;
    }
  };

  return (
    <MenuContext.Provider value={ctx}>
      <Popover
        ref={ref}
        open={open}
        onOpenChange={onOpenChange}
        anchorRef={anchorRef}
        side={side}
        align={align}
        offset={offset}
        matchWidth={matchWidth}
        role="menu"
        ariaLabel={ariaLabel}
        tabIndex={-1}
        onKeyDown={onKeyDown}
        className={cn("w-56", className)}
        {...props}
      >
        <div ref={panelRef} tabIndex={-1} className="outline-none">
          {children}
        </div>
      </Popover>
    </MenuContext.Provider>
  );
});

/* ── rows ─────────────────────────────────────────────────────────────────── */

/**
 * `danger` renders red ink and must be the LAST item in its group, so an
 * irreversible action is never the neighbour of a mis-click.
 */
export const MenuItem = React.forwardRef(function MenuItem(
  { icon, children, description, shortcut, danger, disabled, checked, onSelect, className, ...props },
  ref
) {
  const menu = useMenu();
  const checkable = typeof checked === "boolean";

  const handleSelect = (e) => {
    if (disabled) return;
    onSelect?.(e);
    if (!e.defaultPrevented) menu?.closeAll();
  };

  return (
    <button
      ref={ref}
      type="button"
      role={checkable ? "menuitemcheckbox" : "menuitem"}
      aria-checked={checkable ? checked : undefined}
      aria-disabled={disabled || undefined}
      data-disabled={disabled ? "" : undefined}
      tabIndex={-1}
      disabled={disabled}
      onClick={handleSelect}
      // keep pointer and keyboard highlight on the same row
      onPointerEnter={(e) => !disabled && e.currentTarget.focus({ preventScroll: true })}
      // A row with a second line is taller than the fixed 32px, and its icon and
      // tick belong at the top of it rather than floating in the middle.
      className={cn(itemBase, description && "h-auto items-start py-1.5", danger && dangerInk, className)}
      {...props}
    >
      {renderIcon(
        icon,
        cn("size-3.5", danger && "text-destructive-ink")
      )}
      <span className={cn("flex min-w-0 flex-1 flex-col", description && "gap-0.5")}>
        <span className="truncate">{children}</span>
        {description ? (
          <span className="text-[11px] leading-snug font-normal text-muted-foreground">
            {description}
          </span>
        ) : null}
      </span>
      {shortcut ? (
        <span className="ml-auto shrink-0 pl-2">
          <Kbd>{shortcut}</Kbd>
        </span>
      ) : null}
      {checked ? <Check className="ml-auto size-3.5 text-foreground" /> : null}
    </button>
  );
});

export function MenuLabel({ className, children, ...props }) {
  return (
    <div
      className={cn(
        "px-2.5 pb-1 pt-1.5 text-[11px] font-medium text-muted-foreground",
        className
      )}
      {...props}
    >
      {children}
    </div>
  );
}

export function MenuSeparator({ className, ...props }) {
  // whitespace is the preferred separator - reach for this sparingly
  return (
    <div
      role="separator"
      aria-orientation="horizontal"
      className={cn("-mx-1.5 my-1.5 h-px bg-border-subtle", className)}
      {...props}
    />
  );
}

export function MenuGroup({ label, className, children, ...props }) {
  return (
    <div role="group" aria-label={label} className={cn(className)} {...props}>
      {label ? <MenuLabel>{label}</MenuLabel> : null}
      {children}
    </div>
  );
}

/* ── submenu ──────────────────────────────────────────────────────────────── */

/** `value` (optional) shows the current selection inline, so the parent menu
 *  reads without opening anything. */
export function SubMenu({ label, icon, value, children, className, panelClassName }) {
  const parent = useMenu();
  const triggerRef = React.useRef(null);
  const [open, setOpen] = React.useState(false);
  const closeTimer = React.useRef(null);

  React.useEffect(() => () => clearTimeout(closeTimer.current), []);

  const cancelClose = () => clearTimeout(closeTimer.current);
  const scheduleClose = () => {
    cancelClose();
    closeTimer.current = setTimeout(() => setOpen(false), 120);
  };

  const closeAndFocus = React.useCallback(() => {
    setOpen(false);
    triggerRef.current?.focus({ preventScroll: true });
  }, []);

  const onTriggerKeyDown = (e) => {
    if (e.key === "ArrowRight" || e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      e.stopPropagation();
      setOpen(true);
    }
  };

  const onPanelKeyDown = (e) => {
    if (e.key === "ArrowLeft") {
      e.preventDefault();
      e.stopPropagation();
      closeAndFocus();
    }
  };

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        role="menuitem"
        aria-haspopup="menu"
        aria-expanded={open}
        data-state={open ? "open" : "closed"}
        tabIndex={-1}
        onClick={() => setOpen((v) => !v)}
        onKeyDown={onTriggerKeyDown}
        onPointerEnter={(e) => {
          cancelClose();
          e.currentTarget.focus({ preventScroll: true });
          setOpen(true);
        }}
        onPointerLeave={scheduleClose}
        className={cn(
          itemBase,
          // fill-secondary, NOT bg-muted: an alpha fill darkens whatever it
          // sits on, so the open row deepens exactly the way the hover row
          // does. A named surface cannot - muted is lighter than the light
          // overlay, so an open submenu trigger lit UP while hovering it went
          // down, and the row flickered between the two on the way in.
          "data-[state=open]:fill-secondary data-[state=open]:text-foreground",
          className
        )}
      >
        {renderIcon(icon, "size-3.5")}
        <span className="min-w-0 flex-1 truncate">{label}</span>
        {value ? (
          <span className="ml-auto max-w-[8rem] shrink-0 truncate text-xs text-muted-foreground">
            {value}
          </span>
        ) : null}
        <ChevronRight className="ml-auto size-3.5 shrink-0" />
      </button>

      <MenuPanel
        open={open}
        onOpenChange={setOpen}
        onCloseAll={() => parent?.closeAll?.()}
        anchorRef={triggerRef}
        side="right"
        align="start"
        offset={6}
        ariaLabel={typeof label === "string" ? label : undefined}
        className={panelClassName}
        onKeyDown={onPanelKeyDown}
        onPointerEnter={cancelClose}
        onPointerLeave={scheduleClose}
      >
        {children}
      </MenuPanel>
    </>
  );
}

/* ── the menu itself ──────────────────────────────────────────────────────── */

/**
 * `trigger` accepts BOTH shapes and we detect at runtime:
 *   • a render prop - `({ ref, open, toggle, props }) => node`, when you need
 *     the state (a caret that rotates, a label that changes);
 *   • any single element - cloned with the anchor ref, `data-state` and the
 *     aria wiring, which covers Button/IconButton unchanged.
 */
export function DropdownMenu({
  trigger,
  children,
  side = "bottom",
  align = "start",
  offset = 8,
  className,
  panelClassName,
  open: openProp,
  onOpenChange,
  ariaLabel,
}) {
  const anchorRef = React.useRef(null);
  const [uncontrolled, setUncontrolled] = React.useState(false);
  const isControlled = openProp !== undefined;
  const open = isControlled ? openProp : uncontrolled;

  const setOpen = React.useCallback(
    (next) => {
      if (!isControlled) setUncontrolled(next);
      onOpenChange?.(next);
    },
    [isControlled, onOpenChange]
  );

  // Escape / select must hand focus back to the control that opened the menu.
  const wasOpen = React.useRef(open);
  React.useEffect(() => {
    if (wasOpen.current && !open) anchorRef.current?.focus?.({ preventScroll: true });
    wasOpen.current = open;
  }, [open]);

  const toggle = React.useCallback(() => setOpen(!open), [setOpen, open]);

  const onTriggerKeyDown = (e) => {
    if (!open && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
      e.preventDefault();
      setOpen(true);
    }
  };

  const triggerProps = {
    ref: anchorRef,
    "aria-haspopup": "menu",
    "aria-expanded": open,
    "data-state": open ? "open" : "closed",
    onClick: toggle,
    onKeyDown: onTriggerKeyDown,
  };

  const triggerNode =
    typeof trigger === "function"
      ? trigger({ ref: anchorRef, open, toggle, props: triggerProps })
      : React.isValidElement(trigger)
        ? React.cloneElement(trigger, triggerProps)
        : trigger;

  return (
    <div className={cn("relative inline-flex", className)}>
      {triggerNode}
      <MenuPanel
        open={open}
        onOpenChange={setOpen}
        anchorRef={anchorRef}
        side={side}
        align={align}
        offset={offset}
        ariaLabel={ariaLabel}
        className={panelClassName}
      >
        {children}
      </MenuPanel>
    </div>
  );
}
