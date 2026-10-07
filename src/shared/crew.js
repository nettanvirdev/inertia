/**
 * A team of agents, and the rules about who may build one.
 *
 * Delegation used to be one shape and one shape only: `task` started a
 * subagent, blocked until it answered, and handed back its report. That is a
 * good shape for the thing it was written for - reading forty files to answer
 * one question without those forty files landing in the parent's context - and
 * it is the wrong shape for everything else. A parent that wants research, a
 * UI and tests done at once gets them one after another. A subagent that
 * discovers it needs its own helper cannot have one. And nobody watching can
 * see anything until the whole thing is over.
 *
 * So there are now two shapes. `task` stays exactly as it was, because "go and
 * read this, tell me the answer" is still the common case and blocking on it is
 * the simplest correct thing. Alongside it, `spawn` starts a run and returns an
 * id immediately, `collect` waits on ids when the parent actually needs the
 * answers, and `team` says what everyone is doing without waiting for anyone.
 * The parent keeps working in between, which is the whole point.
 *
 * This file is the part both halves need to agree on: who is allowed to
 * spawn what, and what a run's status means. The runtime lives in the Rust
 * backend (`src-tauri/src/crew.rs`) because that is where sessions run; the
 * panel lives in the window because that is where someone is watching.
 * Neither may hold its own opinion about whether a nested spawn is permitted.
 */

/**
 * What an agent may do about building a team.
 *
 * Defaults chosen so an existing workspace behaves the way it did yesterday
 * and no further: every agent could already delegate one level deep, so
 * `subagents` is on; none of them could nest, spawn a peer, or run unbounded,
 * so those are off and stay off until someone turns them on for that agent.
 *
 * `maxConcurrent` is a count of this agent's own live children, not of the
 * tree. Zero means unlimited, which is a real answer rather than a missing
 * one: the request was explicitly for no artificial ceiling, and the ceiling
 * that matters - money - is reported rather than guessed at.
 */
export const SPAWN_DEFAULTS = {
  /** May hand work to a temporary subagent at all. */
  subagents: true,
  /** May start another configured agent as a peer, not only as a helper. */
  agents: false,
  /** May its children spawn children of their own. */
  recursive: false,
  /** How many of its own children may be live at once. 0 is unlimited. */
  maxConcurrent: 0,
};

/**
 * There is no `requiresApproval` here, and that is not an omission.
 *
 * Delegating already goes through the permission layer under the `task` key, so
 * anyone who wants to be asked before an agent spawns can already set that rule
 * - per agent, and per agent-being-spawned. A second switch meaning the same
 * thing would be a setting that either does nothing or quietly contradicts the
 * rule beside it, and the last thing this app needed was another control that
 * looks like it decides something.
 */

/** One agent's policy, whatever shape the record on disk is in. */
export function spawnPolicy(agent) {
  const stored = agent?.spawn;
  if (!stored || typeof stored !== "object") return { ...SPAWN_DEFAULTS };
  const max = Number(stored.maxConcurrent);
  return {
    subagents: stored.subagents !== false,
    agents: Boolean(stored.agents),
    recursive: Boolean(stored.recursive),
    maxConcurrent: Number.isFinite(max) && max > 0 ? Math.floor(max) : 0,
  };
}

/**
 * May this turn hold the team tools at all?
 *
 * Two different questions wearing one word. At the top of a conversation the
 * question is whether this agent delegates; below it, the question is whether
 * it may nest - and those are separate switches because the failure modes are
 * different. A parent that delegates too eagerly costs a few sessions. A tree
 * that nests without a limit costs a tree.
 */
export function mayDelegate(agent, depth = 0) {
  const policy = spawnPolicy(agent);
  if (depth <= 0) return policy.subagents;
  return policy.subagents && policy.recursive;
}

/**
 * The ceiling on a whole conversation, whatever each agent's policy says.
 *
 * Per-agent `maxConcurrent` defaults to unlimited on purpose. This is the
 * other kind of limit: not "how many may this agent run" but "how many may
 * exist at all under one chat", and it is there for the case no policy
 * anticipates - a coordinator that spawns a helper per file in a repository
 * of three hundred files, or a helper that spawns a helper that spawns a
 * helper. Claude Code's numbers, near enough, and both are far above what
 * useful work needs: twenty things running at once is already more than
 * anyone can review.
 */
export const TEAM_LIMITS = {
  /** Runs in flight at once, across the whole conversation. */
  live: 20,
  /** Runs ever started in one conversation, including finished ones. */
  total: 100,
};

/**
 * Why a particular spawn cannot happen, or null if it can.
 *
 * The sentence is written for the model, because the model is who reads it and
 * the model is who has to do something else instead. "Denied" tells it nothing;
 * "you may not nest, so do this one yourself" tells it what to do next.
 */
export function spawnRefusal(
  agent,
  { depth = 0, running = 0, peer = false, live = 0, total = 0 } = {}
) {
  const policy = spawnPolicy(agent);
  const name = agent?.name ?? "This agent";
  if (total >= TEAM_LIMITS.total) {
    return `This conversation has already started ${total} runs, which is the most one conversation may. Finish with what exists, or ask the user to start a fresh conversation for the rest.`;
  }
  if (live >= TEAM_LIMITS.live) {
    return `There are already ${live} runs in flight in this conversation, which is the most that may run at once. Call \`wait\` or \`collect\` until some finish before spawning more.`;
  }
  if (!policy.subagents) {
    return `${name} is not allowed to delegate work. Do it yourself, or ask the user to turn on subagents for this agent under Agents.`;
  }
  if (depth > 0 && !policy.recursive) {
    return `You are already a subagent, and ${name} is not allowed to spawn subagents of its own. Do this part yourself and report back.`;
  }
  if (peer && !policy.agents) {
    return `${name} may spawn helpers but not full agents. Spawn this as a subagent instead, or ask the user to allow spawning agents.`;
  }
  if (policy.maxConcurrent && running >= policy.maxConcurrent) {
    return `${name} already has ${running} runs in flight, which is its limit. Call \`collect\` on one of them before spawning another.`;
  }
  return null;
}

/**
 * Settled, but with its conversation kept, so a new brief can pick it up.
 *
 * `done` and `failed` keep theirs by construction; `interrupted` is the
 * status of a run that was stopped mid-turn for that purpose - as opposed to
 * `cancelled`, which is stopped for good and whose subtree went with it.
 */
export function canFollowUp(status) {
  return status === "done" || status === "failed" || status === "interrupted";
}

/**
 * Not finished. Its answer is still coming, even if nothing is happening.
 *
 * A paused run counts, and that matters in three places that would each be
 * wrong without it: it still holds one of its parent's concurrency slots, it is
 * still something `collect` is entitled to wait for, and a turn must not end
 * while one is sitting there - a run someone paused and forgot is exactly the
 * case that should not be silently abandoned.
 */
export function isActive(status) {
  return status === "queued" || status === "running" || status === "paused";
}

/**
 * Actually doing something right now.
 *
 * The narrower question, and the one the header dot asks: "one agent working"
 * on a run that is paused is a lie a glance cannot catch.
 */
export function isWorking(status) {
  return status === "queued" || status === "running";
}

/** Finished one way or another - the answer is not going to change. */
export function isSettled(status) {
  return !isActive(status);
}

/**
 * A short present-tense phrase for what a run is doing right now.
 *
 * Derived from the tool it is calling rather than asked for, because a model
 * asked to report its own status reports it once and then forgets. The mapping
 * is coarse on purpose: the panel is glanced at, not read.
 */
/**
 * "Two runs you started have finished" - said to the parent, mid-turn.
 *
 * The gap this closes: `spawn` returns at once and the parent keeps working,
 * which is the point of it, but until now the parent found out that a helper
 * had finished only by asking - `team`, `collect` or `wait`. A model deep in
 * its own share of the work does not ask, so the answers sat there until the
 * end of the turn, and the parent either forgot them or waited for work that
 * had been done for four minutes.
 *
 * So the loop tells it, once per run, at the boundary between two rounds of
 * tool calls. Once per run and never repeated: a note that arrives twice is
 * noise, and a model that ignored it the first time is not going to collect on
 * the second telling.
 *
 * Returns null when there is nothing to say, so the caller has no branch of
 * its own.
 */
export function settledNotice(runs) {
  const list = (runs ?? []).filter(Boolean);
  if (!list.length) return null;

  const described = list
    .map((run) => {
      const how =
        run.status === "done"
          ? "finished"
          : run.status === "failed"
            ? "failed"
            : run.status === "cancelled"
              ? "was cancelled"
              : run.status === "interrupted"
                ? "was interrupted"
                : `is ${run.status}`;
      return `${run.id} (${run.description ?? "no description"}) ${how}`;
    })
    .join("; ");

  return (
    `A note from the system, not from the person you are talking to. ` +
    `${list.length === 1 ? "A run you started has" : `${list.length} runs you started have`} ` +
    `settled: ${described}. Carry on with what you were doing; call \`collect\` on ` +
    `${list.length === 1 ? "it" : "them"} when you need what ${list.length === 1 ? "it" : "they"} ` +
    `found, and fold the answer into your work rather than repeating it yourself.`
  );
}

export function activityFor(toolId) {
  const id = String(toolId ?? "");
  if (!id) return "Thinking";
  if (id === "read" || id === "ls" || id === "glob" || id === "grep") return "Reading";
  if (id === "write" || id === "edit" || id === "patch") return "Writing";
  if (id === "shell" || id.startsWith("shell_")) return "Running commands";
  if (id.startsWith("computer")) return "Driving a computer";
  if (id === "spawn" || id === "task" || id === "collect" || id === "team") return "Coordinating";
  if (id === "interrupt" || id === "followup") return "Coordinating";
  if (id === "wait") return "Waiting on the team";
  if (id === "agent_send") return "Messaging";
  if (id === "question" || id === "present_plan") return "Waiting on the user";
  if (id === "todowrite" || id === "skill") return "Planning";
  return "Working";
}
