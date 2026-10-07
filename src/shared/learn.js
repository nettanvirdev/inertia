/**
 * Turning what just happened into something the next agent already knows.
 *
 * The app has two kinds of durable memory and they answer different questions.
 * A MEMORY is a fact - "the workspace is ~/Documents/Inertia", "he prefers no
 * attribution lines in commits" - noticed after a conversation goes quiet, by
 * the capture pass in the `inertia-memory` crate, without anybody asking. A
 * SKILL is a procedure: how to do a thing, with the commands, the order, and
 * the traps. Nothing ever wrote
 * one, because nothing ever asked - the tools to save one have been there all
 * along and an agent has no reason to reach for them mid-task.
 *
 * `/learn` is the asking. Borrowed from Hermes, including the part that makes
 * it cheap: there is no distillation engine and no second model. It builds one
 * prompt and sends it as an ordinary turn, so the agent gathers what it needs
 * with the tools it already has and writes the skill with `inertia_save`. That
 * means it works identically wherever a turn works, and there is one code path
 * to be wrong rather than two.
 *
 * The standards below are most of the value. A skill nobody can route to is a
 * skill that does not exist, and the description is the whole routing signal:
 * it is what every agent sees, every session, before deciding whether to load
 * the body. Left to itself a model writes "A comprehensive skill for managing
 * deployments across environments" - which says nothing, costs tokens in every
 * prompt, and never matches a question.
 */

/**
 * The house rules, as the agent is told them.
 *
 * Framed as what a reviewer would send back, because that is the register a
 * model follows most reliably: not "try to be concise" but "count the
 * characters, and cut it if it is over".
 */
const STANDARDS = `Write it to this standard. These are the rules a reviewer would send it back for:

**name** - lowercase, hyphenated, no spaces. What the thing IS, not what it is about: \`release-a-build\`, not \`release-notes-helper\`.

**description** - ONE sentence, at most 100 characters, ending in a full stop. This is the only part of the skill any agent sees before deciding whether to load it, in every prompt, in every session - so it is routing text, not a summary. State the capability plainly and name the concrete nouns somebody would ask about. No marketing words: comprehensive, powerful, seamless, advanced, robust, helper, utility. Do not repeat the skill's own name in it. Count the characters when you have written it; if it is over 100, cut it rather than hoping.
  Good: \`Cut a Windows release: version bump, icons, electron-builder, GitHub upload.\`
  Bad:  \`A comprehensive skill for managing the release process end to end.\`

**instructions** - the body. Sections in this order, and leave one out only if it genuinely has nothing in it:
1. One short paragraph: what this does, what it deliberately does NOT do, and anything it depends on.
2. **When to use** - the concrete phrasings somebody would actually type.
3. **Before you start** - the credentials, environment variables, installed tools it needs, exactly named.
4. **Steps** - numbered, with commands copy-paste exact. Say which tool runs each one.
5. **Traps** - what looks broken and is not, what fails silently, the limits.
6. **Checking it worked** - one command or one observation that proves it.

Quality bar, and this is the part that decides whether the skill is worth its tokens:
- Every command, path, flag, URL and config key must be one you actually SAW in this conversation or in the source. Never invent a flag or a path. If you did not see it, leave it out and say so under Traps.
- Write down what was expensive to find out: the error and its real cause, the command that finally worked and why the obvious one did not, the value that had to be exactly right. Not what any competent person would guess.
- Keep it tight. Around a hundred lines for something simple, two hundred for something involved. It is a procedure, not a tutorial.
- Do not write a skill that only points at other skills.
- If it is specific to one machine or one folder, say so in the first paragraph. A skill that silently assumes a path is a skill that fails on the next machine.`;

/**
 * What `/learn` says, for whatever the person pointed at.
 *
 * With no argument it means this conversation, which is the case worth
 * optimising for: the moment somebody says "learn this" is the moment
 * something has just worked, and the transcript is right there.
 */
export function learnPrompt(subject = "") {
  const source = String(subject ?? "").trim();

  const target = source
    ? `Write a skill covering this:\n\n${source}\n\nGather what you need first - read the files, open the pages, run the commands - using the tools you have. Do not write the skill from memory or from guesswork.`
    : `Write a skill covering what we just did in this conversation.\n\nRead back over it first. The skill is the PROCEDURE that worked, not a diary: what the goal was, the steps that got there in the order they have to happen, the commands exactly as they were run, and every dead end worth warning the next agent about. If part of it was luck or is still unfinished, say so under Traps rather than writing it up as though it works.`;

  return [
    target,
    "",
    STANDARDS,
    "",
    "Save it with `inertia_save` as a skill, with `name`, `description` and `instructions`. If a skill already covers this, read it with `inertia_read` and update that one rather than adding a second - two skills for one job is how a workspace stops routing to either.",
    "",
    "When it is saved, tell me its name and description and the one thing you were least sure about. Nothing else - no summary of the skill, I can read it.",
  ].join("\n");
}

export { STANDARDS as LEARN_STANDARDS };
