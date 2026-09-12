import * as React from "react";
import { useApp } from "@/lib/store";
import { useToast } from "@/components/ui/toast";
import { copyText } from "@/lib/clipboard";
import { helpText, parseCommand, readToggle } from "@shared/commands";
import { learnPrompt } from "@shared/learn";
import { addUsage, costOf, formatCost, formatTokens, priceFor, totalTokens } from "@shared/usage";

/**
 * What a slash command actually does.
 *
 * The table of names lives in `shared/commands`, so the palette and this file
 * cannot disagree about what exists; the behaviour lives here, where the store
 * is. Nothing here talks to a model except `/compact`, which is the one
 * command that costs anything.
 *
 * Two rules, both about not surprising anyone:
 *
 * - A command is answered IN the conversation, as a grey line, and not as a
 *   toast that is gone before it has been read. `/context` is a number
 *   somebody wants to look at while deciding what to type next.
 * - A command never becomes a message. `run` returns true when it took the
 *   text, and the composer then clears the box without sending anything - so
 *   `/clear` does not leave "/clear" behind in the conversation it just left.
 *
 * The one exception is a command that IS a message: `/learn` has no machinery,
 * it just puts a carefully written instruction in front of the agent. Those
 * return the text to send, and the composer sends it with the model and mode
 * its own pickers are showing - which is why the substitution happens there
 * rather than by calling `sendMessage` from in here.
 */
export function useCommands({ threadId, onModel } = {}) {
  const {
    threads,
    messages,
    agents,
    chatModels,
    modelPrices,
    user,
    createThread,
    renameThread,
    stopMessage,
    compactThread,
    noteThread,
    setView,
    openSettings,
    setPreference,
    startingAgentId,
  } = useApp();
  const { toast } = useToast();

  return React.useCallback(
    (text) => {
      const parsed = parseCommand(text);
      if (!parsed) return false;

      const { name, args } = parsed;
      const thread = threads.find((t) => t.id === threadId) ?? null;
      const say = (line) => noteThread(threadId, line);

      switch (name) {
        case "learn":
          // Returned rather than sent: see the note above. The composer owns
          // which model and which mode a message goes out under, and a turn
          // started from here would quietly ignore both of its pickers.
          return learnPrompt(args);

        case "help":
          say(`The commands this box understands:\n\n${helpText()}`);
          return true;

        case "compact":
          // Not awaited. It is a round trip to a model and the composer has to
          // come back immediately; the note it leaves behind says it started,
          // and is rewritten in place when it finishes.
          compactThread(threadId, args);
          return true;

        case "autocompact": {
          const wanted = readToggle(args);
          const now = user?.preferences?.autoCompact !== false;
          if (wanted === null) {
            say(
              now
                ? "Automatic compaction is on: a conversation that nearly fills the model's window is summarised once, visibly. Turn it off with /autocompact off."
                : "Automatic compaction is off, and nothing else summarises for you: a conversation that outgrows the window will be refused by the provider rather than shortened. Run /compact yourself, or turn this back on with /autocompact on."
            );
            return true;
          }
          setPreference("autoCompact", wanted);
          say(wanted ? "Automatic compaction is on." : "Automatic compaction is off.");
          return true;
        }

        case "clear":
          createThread(thread?.agentId ?? startingAgentId);
          return true;

        case "context": {
          const used = Number(thread?.context?.used) || 0;
          const window = Number(thread?.context?.window) || 0;
          if (!window) {
            say("Nothing has been sent to a model in this conversation yet, so there is no context to measure.");
            return true;
          }
          const percent = Math.round((used / window) * 100);
          say(
            `Context: ${formatTokens(used)} of ${formatTokens(window)} tokens, ${percent}% full. ` +
              (percent >= 85
                ? "The next turn will be summarised."
                : "There is room. /compact summarises it early if you would rather carry on from a clean note.")
          );
          return true;
        }

        case "cost": {
          // Added up from what each reply reported. A turn whose provider says
          // nothing about usage contributes nothing, which is why this says
          // what was reported rather than pretending to be a bill.
          let usage = null;
          let replies = 0;
          for (const message of messages[threadId] ?? []) {
            if (!message?.usage) continue;
            usage = addUsage(usage, message.usage);
            replies += 1;
          }
          if (!replies) {
            say("No model has reported what it used in this conversation yet.");
            return true;
          }
          const agent = agents.find((a) => a.id === thread?.agentId);
          const modelId = chatModels.find((m) => m.ref === agent?.model)?.id ?? agent?.model ?? "";
          const price = priceFor(modelId, modelPrices);
          const dollars = price ? costOf(usage, price) : null;
          say(
            `${formatTokens(totalTokens(usage))} tokens over ${replies} ${replies === 1 ? "reply" : "replies"}` +
              (dollars != null ? `, about ${formatCost(dollars)}.` : ". No price is known for this model.")
          );
          return true;
        }

        case "model": {
          const wanted = args.trim().toLowerCase();
          if (!wanted) {
            const agent = agents.find((a) => a.id === thread?.agentId);
            const current = chatModels.find((m) => m.ref === agent?.model);
            say(
              current
                ? `This conversation is using ${current.label ?? current.id}. Type /model and part of a name to change it.`
                : "This conversation uses whichever model its agent is set to. Add one under Settings, Models."
            );
            return true;
          }
          const match =
            chatModels.find((m) => String(m.id ?? "").toLowerCase() === wanted) ??
            chatModels.find((m) => String(m.label ?? "").toLowerCase().includes(wanted)) ??
            chatModels.find((m) => String(m.id ?? "").toLowerCase().includes(wanted));
          if (!match) {
            say(
              `No configured model matches "${args.trim()}". The ones you have: ${
                chatModels.map((m) => m.id).join(", ") || "none yet"
              }.`
            );
            return true;
          }
          onModel?.(match.ref);
          say(`The next message goes to ${match.label ?? match.id}.`);
          return true;
        }

        case "rename": {
          const title = args.trim();
          if (!title) {
            say("Give it a name: /rename The Collatz room");
            return true;
          }
          renameThread(threadId, title);
          return true;
        }

        case "export": {
          const lines = (messages[threadId] ?? [])
            .filter((message) => message?.role !== "system")
            .map((message) => {
              const who =
                message.role === "user"
                  ? (user?.name ?? "You")
                  : (agents.find((a) => a.id === message.agentId)?.name ?? "Agent");
              return `## ${who}\n\n${message.content ?? ""}`;
            });
          if (!lines.length) {
            say("There is nothing in this conversation to copy yet.");
            return true;
          }
          copyText(`# ${thread?.title ?? "Conversation"}\n\n${lines.join("\n\n")}`)
            .then(() => toast({ title: "Conversation copied" }))
            .catch(() => toast({ title: "The conversation could not be copied", variant: "danger" }));
          return true;
        }

        case "stop":
          stopMessage(threadId);
          return true;

        case "agents":
          setView("agents");
          return true;

        case "memory":
          setView("memory");
          return true;

        case "permissions":
          openSettings("permissions");
          return true;

        case "settings":
          openSettings(args.trim() || undefined);
          return true;

        default:
          return false;
      }
    },
    [
      threadId,
      threads,
      messages,
      agents,
      chatModels,
      modelPrices,
      user,
      createThread,
      renameThread,
      stopMessage,
      compactThread,
      noteThread,
      setView,
      openSettings,
      setPreference,
      startingAgentId,
      onModel,
      toast,
    ]
  );
}
