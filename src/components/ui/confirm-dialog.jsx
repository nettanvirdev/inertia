import * as React from "react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Dialog, DialogTitle, DialogDescription } from "@/components/ui/dialog";

/**
 * Every irreversible action goes through here.
 *
 * `destructive` does NOT paint the confirm button red: in this system the loud
 * button is the neutral-inverted primary, and a red slab would be the loudest
 * thing on a flat screen. It only shifts the default focus onto Cancel, so the
 * dangerous key is never the one already under the finger.
 */
export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  destructive = false,
  onConfirm,
  loading = false,
  className,
}) {
  const cancelRef = React.useRef(null);

  React.useEffect(() => {
    if (!open || !destructive) return;
    const raf = requestAnimationFrame(() => cancelRef.current?.focus({ preventScroll: true }));
    return () => cancelAnimationFrame(raf);
  }, [open, destructive]);

  const cancel = () => onOpenChange?.(false);

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      size="sm"
      showClose={false}
      closeOnOverlay={!loading}
      ariaLabel={typeof title === "string" ? title : "Confirm"}
      className={cn("w-[calc(100%-1.5rem)]", className)}
    >
      <DialogTitle>{title}</DialogTitle>
      {description ? <DialogDescription>{description}</DialogDescription> : null}

      <div
        data-destructive={destructive ? "true" : undefined}
        className="mt-5 flex flex-col-reverse gap-1.5 md:flex-row"
      >
        <Button
          ref={cancelRef}
          variant="secondary"
          size="pill"
          className="w-full md:min-w-0 md:flex-1"
          disabled={loading}
          onClick={cancel}
        >
          {cancelLabel}
        </Button>
        <Button
          variant="primary"
          size="pill"
          className="w-full md:min-w-0 md:flex-1"
          disabled={loading}
          onClick={onConfirm}
        >
          {loading ? <Spinner size="sm" /> : null}
          {confirmLabel}
        </Button>
      </div>
    </Dialog>
  );
}
