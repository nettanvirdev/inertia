import * as React from "react";
import { cn } from "@/lib/utils";

/** One calm sentence. No illustration, no bordered box, no CTA unless there is genuinely one thing to do. */
function EmptyState({ icon: Icon, title, description, action, className, ...props }) {
  return (
    <div
      className={cn(
        "flex w-full flex-col items-center justify-center gap-3 px-6 py-10 text-center",
        className
      )}
      {...props}
    >
      {Icon ? (
        <span className="flex size-10 items-center justify-center rounded-full fill-control text-muted-foreground">
          <Icon className="size-5" aria-hidden="true" />
        </span>
      ) : null}
      <div className="flex max-w-80 flex-col gap-1">
        {title ? <p className="text-base font-medium text-foreground">{title}</p> : null}
        {description ? (
          <p className="text-[13px] leading-relaxed text-muted-foreground">{description}</p>
        ) : null}
      </div>
      {action ? <div className="mt-1">{action}</div> : null}
    </div>
  );
}

export { EmptyState };
