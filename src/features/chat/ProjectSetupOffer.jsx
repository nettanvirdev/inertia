import * as React from "react";
import { FileText } from "@/components/icons";
import { useApp } from "@/lib/store";
import { PREFERENCE_DEFAULTS } from "@/lib/appearance";
import {
  createProjectSetup,
  declineProjectSetup,
  decideProjectSetup,
} from "@/lib/project";
import { Button } from "@/components/ui/button";
import { useToast } from "@/components/ui/toast";

/**
 * Offering a project the file that tells every agent how it works.
 *
 * Shown once per folder, above the composer, and only when there is genuinely
 * something to offer: no `AGENTS.md`, no `CLAUDE.md`, no rules folder, and
 * nobody has said no before. Turning it down is remembered - an offer that
 * reappears is one people learn to dismiss without reading, which is worse than
 * never having made it.
 *
 * The decision about whether to appear at all is made in the main process. A
 * second copy of that rule here would eventually disagree with the first, and
 * the disagreement would show up as a repository quietly gaining a file.
 */
export function ProjectSetupOffer({ cwd }) {
  const { user } = useApp();
  const { toast } = useToast();
  const mode = user?.preferences?.projectInit ?? PREFERENCE_DEFAULTS.projectInit;

  const [state, setState] = React.useState(null);
  const [busy, setBusy] = React.useState(false);
  // Dismissed for this view only. The durable "no" is written by `decline`;
  // this is what makes the card disappear the moment it is acted on.
  const [gone, setGone] = React.useState(false);

  React.useEffect(() => {
    let alive = true;
    setGone(false);
    setState(null);
    if (!cwd) return undefined;

    decideProjectSetup(cwd, { mode, moment: "message" })
      .then((answer) => alive && setState(answer))
      .catch(() => alive && setState(null));
    return () => {
      alive = false;
    };
  }, [cwd, mode]);

  /**
   * The modes that write without being asked do it here, once, quietly.
   *
   * Announced with a toast rather than silence: a file appearing in a repository
   * with no indication of where it came from is the thing this whole feature is
   * careful about, and the user did ask for it in settings.
   */
  React.useEffect(() => {
    if (state?.action !== "create" || busy || gone) return;
    let alive = true;
    setBusy(true);
    createProjectSetup(cwd)
      .then((result) => {
        if (!alive) return;
        setGone(true);
        if (result?.created?.length) {
          toast({
            title: "Project notes created",
            description: `${result.created.join(" and ")} in ${result.folder}`,
            variant: "success",
          });
        }
      })
      .catch(
        (error) =>
          alive &&
          toast({
            title: "It could not be set up",
            description: error?.message ?? "That did not work.",
            variant: "danger",
          })
      )
      .finally(() => alive && setBusy(false));
    return () => {
      alive = false;
    };
  }, [state, cwd, busy, gone, toast]);

  if (gone || state?.action !== "ask") return null;

  const accept = async () => {
    setBusy(true);
    try {
      const result = await createProjectSetup(cwd);
      setGone(true);
      toast({
        title: result?.created?.length ? "Project notes created" : "Already set up",
        description: result?.created?.length
          ? `${result.created.join(" and ")}. Open AGENTS.md and tell it about this project.`
          : "This project already says something about itself.",
        variant: "success",
      });
    } catch (error) {
      toast({
        title: "It could not be set up",
        description: error?.message ?? "That did not work.",
        variant: "danger",
      });
    } finally {
      setBusy(false);
    }
  };

  const dismiss = async () => {
    setGone(true);
    await declineProjectSetup(cwd).catch(() => {});
  };

  return (
    <div className="mb-2 flex items-start gap-3 rounded-2xl card-surface-subtle px-3 py-2.5">
      <FileText aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <p className="text-[13px] text-foreground">
          This project has no notes for agents yet.
        </p>
        <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
          An <span className="font-mono">AGENTS.md</span> is read at the start of every conversation,
          so you explain the project once instead of every time. Other coding agents read the same
          file.
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        <Button size="xs" variant="ghost" onClick={dismiss} disabled={busy}>
          Not now
        </Button>
        <Button size="xs" onClick={accept} disabled={busy}>
          Set it up
        </Button>
      </div>
    </div>
  );
}
