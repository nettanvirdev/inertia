import logo from "@setup/logo.png";
import { cn } from "@/lib/utils";

/**
 * The product mark. A raster asset rather than inline SVG because the artwork
 * is gradient-shaded on every facet - redrawing it as paths would approximate
 * it, and an approximation of a logo is worse than no logo. The 512px source
 * covers every size it is drawn at here (56px at 4x DPI is 224px).
 *
 * It lives in the installer rather than in the app's `src/` because `src/` is
 * a mirror of the renderer and gains nothing from a file only setup imports.
 *
 * `size-*` from the caller drives it; the entrance animation is in
 * globals.css so the reduced-motion rules there apply to it too.
 */
export function Mark({ className }: { className?: string }) {
  return (
    <img
      src={logo}
      alt=""
      aria-hidden="true"
      draggable={false}
      className={cn("mark-logo select-none object-contain", className)}
    />
  );
}
