# Agents, sub-agents, skills, memory and routines

## Agents

An agent is a named configuration the conversation runs as. Each one is a file,
`agents/<id>.json`, edited on the **Agents** screen (or by an agent with the
`inertia_save` tool). An agent has:

- **Name, role, description and picture**, shown in the conversation header and
  used by other agents to address it (`@handle`).
- **Instructions** (`systemPrompt`), placed first in the system prompt.
- **Model** (`provider/model`). Without one, the workspace default model is
  used. Thinking settings (a token budget for Anthropic, `low`/`medium`/`high`
  effort for OpenAI-compatible models) are read from the record.
- **Working folder** (`cwd`): where its file and shell tools work by default.
- **Computer** (`computerId`): a machine it can drive with the `computer_*`
  tools. See [computers.md](computers.md).
- **Permissions**: per-agent rules layered on top of the workspace rules. See
  [tools.md](tools.md#permissions).
- **Delegation policy**: whether it may start sub-agents, whether those may
  start their own, whether it may start full agents rather than temporary
  helpers, and how many runs it may have at once.
- **Paused**: a paused agent takes no work, including from routines.

Agents the app seeds are marked `protected`; agents cannot change or delete
them through the `inertia_*` tools.

### Project instructions

At the start of every conversation, `AGENTS.md` and files in `.inertia/rules/`
are read from the repository root down to the working folder and added to the
prompt. `AGENTS.md` is the same file other coding agents read. When you open a
project folder without one, Inertia offers to create it; it never overwrites
an existing file, and declining is remembered.

## Sub-agents

There are two ways for an agent to hand work to another.

**`task`** delegates and waits. The sub-agent gets only the prompt it is given,
works in a session of its own with its own tools, and returns a report. It
cannot see the parent conversation or ask questions, and it cannot call `task`
itself. Use it to keep a large search or a lot of reading out of the main
conversation. Passing a previous `task_id` continues that sub-agent's session.

**The crew** delegates without waiting:

| Tool | |
|---|---|
| `spawn` | start a run and get an id back immediately |
| `collect` | wait for specific runs and read their results |
| `wait` | block until anything happens: a run settles, a message arrives, or you speak |
| `team` | see what every run is doing, and read messages sent to you |
| `agent_send` | leave a message in another run's inbox |
| `interrupt` | stop a run mid-turn but keep its transcript |
| `followup` | give a finished run a new brief, with its context intact |

Runs appear in the crew panel beside the conversation, where you can watch,
stop or follow up on them. Limits: whatever each agent's delegation policy
allows, and never more than 20 live runs or 100 runs in total per conversation.
Runs are held in memory and do not survive a restart.

All delegation tools ask under the `task` permission key, and none of them are
available in Chat or Plan mode, so a turn that may not change anything cannot
ask a helper to change it.

## Group conversations

In **Agent Group** mode several agents share one conversation and one
transcript, and take turns. Mention an agent with `@handle` to bring it in.
Agents can `invite` another agent, `handover` the conversation to one, or
`part` when they are done. **Settings > Agent Group** controls whether agents
may do each of those, how many agents a conversation can hold, and how many
turns agents may take between your messages.

## Skills

A skill is a folder of instructions for a kind of work: `skills/<slug>/SKILL.md`
plus any scripts, templates or reference files it needs.

```markdown
---
name: release-notes
description: Write release notes from merged pull requests. Use when asked for a changelog or release notes.
---

1. List the PRs merged since the last tag ...
```

Only each skill's name and description go into the system prompt. When a task
matches a description, the agent calls `skill` to load the full instructions
and a listing of the folder, and can then read the reference files or run the
scripts the instructions mention. Write the description to say both what the
skill covers and when to use it.

Loading a skill grants no permissions. An `allowed-tools` field in the
frontmatter is passed to the model as advice only.

## Memory

Memories are short, durable facts carried between conversations: how you like
to work, decisions and the reasons for them, project conventions, corrections
you made.

- Each memory is one record with a title, a body, a kind (`fact`,
  `preference`, `contact`, `project`, `credential-note`, `handover`) and a
  scope: **global** (about you, used everywhere) or **project** (about one
  folder, used only there). Project memories are stored in the project's
  `.inertia/memory/`.
- At the start of a turn, the titles of the most relevant memories are added
  to the prompt within a small byte budget. The agent fetches full text with
  `memory_recall`, writes with `memory_save`, and removes with
  `memory_forget`.
- After a conversation goes quiet (and when the app quits), a background pass
  can pick out facts worth keeping.
- The **Memory** screen lists, edits and deletes memories. **Settings >
  Memory** turns global and project memory on or off, chooses when to capture
  and whether to ask before saving, how much to carry into each conversation,
  and where memories are stored: the workspace folder, or an MCP memory server
  (so another coding agent pointed at the same server shares them).

Do not store secrets in memories. They are plain files and are sent to the
model as part of the prompt.

## Routines

A routine is a playbook (Markdown instructions) plus a schedule, run by one
agent in a conversation of its own, `routine-<id>`, visible under
**Routines**. Routines are `routines/<id>.json`.

### Schedules

| `schedule.kind` | `schedule.expression` | Runs |
|---|---|---|
| `manual` | | only when you press Run |
| `cron` | five-field cron, e.g. `0 9 * * 1-5` | on that schedule, in **local time** |
| `interval` | ISO 8601 duration, e.g. `PT30M` | that long after the last run; minimum one minute |
| `once` | an ISO date-time | once at that time; if the app was closed then, as soon as it next runs |

The scheduler runs inside the app process, checks routines every half minute,
and runs one routine at a time. **Routines only run while Inertia is
running**; with minimise-to-tray on, closing the window keeps it running.
Because cron is matched against local time, a run can be skipped or repeated
across a daylight-saving change, as with cron itself.

### Mode and permissions

A routine's **mode** defaults to Autonomous (every tool), since a routine is
work you approved when you wrote it. Chat or Plan restrict it.

Routines follow your permission rules like any conversation. When a routine's
tool call needs approval, the card appears in the routine's conversation and
Inertia shows a system notification. If nobody answers, the run is stopped
after 15 minutes. For a routine to run on its own, the rules must already
allow what it does: add allow rules (workspace-wide, or for the routine's
agent) for the commands and tools it needs.

The playbook is sent with a short preamble telling the agent it is running
unattended and to finish with a summary rather than ask questions. Write
playbooks that decide rather than ask.

Agents can create routines with `inertia_save` and one-off follow-ups with
`later`.

### Running and history

Each run writes into the routine's conversation, where you can open it like
any other. **Run** on the routine's page starts it immediately. A paused agent's
routines are skipped (and run once the agent is resumed). Failures are recorded
in the failure log and are readable by agents with the `failures` tool.
