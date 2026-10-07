/**
 * How many tools a turn carries before it has been asked to do anything.
 *
 * Every tool a turn holds is its schema in the request, and the model reads all
 * of them before it writes a word. Measured against a real provider: the core
 * thirteen cost about seven hundred milliseconds to first token, and the full
 * set with a couple of connected apps costs two to three seconds. That is paid
 * on every turn, including the ones that only needed `read`.
 *
 * So there are two answers, and which is right depends on the work:
 *
 *   · **all** - every tool in every request. The model can act immediately and
 *     never has to ask for anything. Right for long autonomous runs, where one
 *     extra second at the top is lost in the noise.
 *   · **on-demand** - the tools that are used constantly are always there, and
 *     the rest wait behind one small tool that loads them by name. Right for
 *     short exchanges, where the delay before the first word is most of what
 *     the person experiences, and for keeping context small.
 *
 * The trade is honest in both directions and is not a matter of taste: on-demand
 * costs one extra round trip on the turn that first needs a loaded family, and
 * a model reaches for what it can see, so a family behind the gate is used less
 * readily than one in front of it. That is why the core is never behind it.
 */

export const TOOL_ACCESS = [
  {
    id: "all",
    label: "Load every tool",
    hint: "The agent can act at once. Slower to start, and a larger context.",
  },
  {
    id: "on-demand",
    label: "Load tools when they are needed",
    hint: "Faster to start and smaller context. One extra step the first time a group is used.",
  },
];

/** What the app does when nobody has said otherwise. */
export const DEFAULT_TOOL_ACCESS = "all";
