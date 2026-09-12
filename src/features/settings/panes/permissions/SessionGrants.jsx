import * as React from "react";
import { ANY } from "@shared/permission";
import { Check, RotateCcw, X } from "@/components/icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Tooltip } from "@/components/ui/tooltip";
import { useToast } from "@/components/ui/toast";
import { permission as permissionClient } from "@/lib/agent";
import { SettingsCard, SettingsSection } from "../../SettingsRow";
import { ACTION_TONE } from "./RuleEditor";

/**
 * What "always" remembered, and how to take it back.
 *
 * Pressing Always on a prompt writes a rule into memory for that conversation.
 * That is the right scope - a decision made in the middle of one piece of work
 * should not quietly become a permanent setting nobody visited a screen to
 * make - but until now it was also the one permission in the app you could
 * neither see nor undo. You agreed to something once and had no way to find out
 * what, or to change your mind before the conversation ended.
 *
 * So: list them, forget them one at a time, or promote one into the workspace
 * ruleset where it survives the conversation and shows up with all the others.
 */
export function SessionGrants({ threads = [], onPromote }) {
  const { toast } = useToast();
  const [grants, setGrants] = React.useState([]);

  const reload = React.useCallback(async () => {
    try {
      // A browser preview has no bridge, so there is nothing remembered and
      // nothing to show. Absence is the empty case, not an error.
      const rows = await permissionClient.grants?.();
      setGrants(Array.isArray(rows) ? rows : []);
    } catch {
      setGrants([]);
    }
  }, []);

  React.useEffect(() => {
    reload();
  }, [reload]);

  if (!grants.length) return null;

  /** A composite id like `thread-4/task-2` belongs to the thread it started in. */
  const titleFor = (sessionId) => {
    const rootId = String(sessionId ?? "").split("/")[0];
    const thread = threads.find((entry) => entry.id === rootId);
    const suffix = sessionId !== rootId ? " (subagent)" : "";
    return (thread?.title ?? rootId) + suffix;
  };

  async function forget(grant) {
    const result = await permissionClient.revoke?.(grant.sessionId, grant.tool, grant.pattern);
    if (result?.ok === false) {
      toast({ variant: "warning", title: "That grant was already gone" });
    }
    await reload();
  }

  async function promote(grant) {
    onPromote?.(grant);
    // Promoted and remembered at once would be the same rule twice, and the
    // workspace copy is the one that outlives the conversation.
    await permissionClient.revoke?.(grant.sessionId, grant.tool, grant.pattern);
    await reload();
    toast({
      variant: "success",
      title: "Added to the workspace rules",
      description: "It now applies to every conversation, not just this one.",
    });
  }

  return (
    <SettingsSection flat
      title="Remembered in a conversation"
      description="Rules you granted by pressing Always on a prompt. They last until that conversation ends and apply to nothing else. Keep one for good and it moves up into the list above."
    >
      <SettingsCard className="flex flex-col gap-0.5 p-1.5">
        {grants.map((grant) => (
          <div
            key={`${grant.sessionId} ${grant.tool} ${grant.pattern}`}
            className="flex animate-slide-up items-center gap-2 rounded-lg px-2 py-1.5 transition-colors duration-150 ease-out hover:fill-control-hover"
          >
            <Badge variant={ACTION_TONE[grant.action]} size="sm" dot>
              {grant.action}
            </Badge>
            <p className="min-w-0 flex-1 truncate font-mono text-xs text-foreground">
              {grant.tool}
              {grant.pattern === ANY ? "" : ` ${grant.pattern}`}
            </p>
            <p className="hidden min-w-0 max-w-[16rem] shrink-0 truncate text-[0.6875rem] text-muted-foreground sm:block">
              {titleFor(grant.sessionId)}
            </p>
            <Button
              size="xs"
              variant="subtle"
              className="shrink-0"
              onClick={() => promote(grant)}
              title="Move this into the workspace rules, where it applies everywhere"
            >
              <Check />
              Keep for good
            </Button>
            <Tooltip content="Forget this grant">
              <IconButton
                size="sm"
                label={`Forget the grant for ${grant.tool}`}
                onClick={() => forget(grant)}
              >
                <X />
              </IconButton>
            </Tooltip>
          </div>
        ))}
      </SettingsCard>

      <div className="flex">
        <Button size="xs" variant="ghost" className="px-1.5 text-muted-foreground" onClick={reload}>
          <RotateCcw />
          Refresh
        </Button>
      </div>
    </SettingsSection>
  );
}
