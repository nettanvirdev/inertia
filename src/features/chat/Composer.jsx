import * as React from "react";
import {
  AtSign,
  CalendarClock,
  ChevronDown,
  Camera,
  Folder,
  FolderOpen,
  FolderTree,
  GitBranch,
  Mic,
  Monitor,
  Paperclip,
  Plus,
  SendArrow,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  Square,
  X,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { MODELS, PROVIDERS } from "@/data";
import { useApp } from "@/lib/store";
import { Collapse } from "@/components/ui/collapse";
import { IconButton } from "@/components/ui/icon-button";
import { Select } from "@/components/ui/select";
import { DropdownMenu, MenuItem, MenuLabel, MenuSeparator } from "@/components/ui/dropdown-menu";
import { useToast } from "@/components/ui/toast";
import { computers as machines } from "@/lib/computers";
import { useWorkspace } from "@/lib/workspace";
import { basename, useWorkingFolder } from "@/hooks/use-working-folder";
import { formatBytes, readAttachment } from "./attachments";
import { useDictation } from "@/hooks/use-dictation";
import { useVoiceSettings } from "@/hooks/use-voice-settings";
import { voice as voiceClient } from "@/lib/voice";
import { MODES as MODE_LIST, DEFAULT_MODE } from "@shared/modes";
import { APPROVALS, DEFAULT_APPROVAL } from "@shared/approval";
import { useDraft } from "@/lib/drafts";
import { useCollection } from "@/lib/workspace";
import { PromptEditor } from "./PromptEditor.jsx";
import { MentionPalette, useMentionPalette } from "./MentionPalette.jsx";
import { mentionSpecs } from "./mentions.js";
import { COMMANDS } from "@shared/commands";
import { useCommands } from "./use-commands.js";
import { useSwap } from "./arrival.js";
import { renderToStaticMarkup } from "react-dom/server";
import { Icon } from "@/components/icons";
import { cachedAvatarSrc } from "@/lib/avatar";

/**
 * The composer: one surface holding an editor and a toolbar, not an editor with
 * a toolbar strip bolted under it.
 *
 * No hairline in any state. The one edge left in the app is the one around a
 * dropdown sheet; everything else, this included, is told apart by its fill.
 */

/**
 * The picker's rows: the three real modes, then Temporary.
 *
 * Temporary is not a mode - it says whether the conversation exists after you
 * leave it, not what the agent may do - but it is picked here, under a
 * separator, and while it is on the pill says so. That is what the person
 * reads the pill for: "Temporary" on it is the one glance that tells them
 * nothing here is being kept.
 *
 * It is offered before the first message and never after, and it cannot be
 * turned off once the conversation has started. Not as a courtesy - as the
 * only way it can mean anything. A conversation that could later be kept was
 * never temporary; and the rule the whole app runs on is that the folder
 * wins, so a conversation already written there cannot be un-written by a
 * flag in this window. Before the first message there is nothing on disk to
 * come back, which is why this is the one moment the choice sticks.
 */
const MODES = MODE_LIST.map((mode) => ({
  value: mode.id,
  label: mode.label,
  hint: mode.hint,
}));

const TEMPORARY = {
  label: "Temporary",
  hint: "This one won’t be saved.",
};

/**
 * One recipe for every pill in the toolbar row.
 *
 * Mode, approval, folder and the model select were each carrying their own
 * copy of "a small round control", and the copies had drifted: one lit up on
 * open and the others did not, one used `bg-muted` where the rest used the
 * control fill, one sat at 12px while the rest sat at 11. Four controls doing
 * the same job, four different colours in the same row.
 *
 * Open is deliberately the same weight as hover. The menu itself is the loud
 * signal that a picker is open; the trigger only has to stay lit.
 */
const PILL = cn(
  // `whitespace-nowrap` because these labels are phrases, not words:
  // "Accept edits" broke over two lines inside a pill one line tall the
  // moment the row was tight enough to squeeze it.
  "hidden h-[1.875rem] max-w-44 items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 text-[11px] outline-none lg:flex",
  // Secondary ink, not muted. Muted is the shade for text nobody has to
  // read; these pills say which model, which folder, which mode - the
  // answers someone checks at a glance - and at 11px on a 3%-ink fill the
  // muted grey was too faint to check anything by, in either theme.
  "fill-control text-foreground-secondary transition-colors duration-150 ease-out",
  "hover:fill-control-hover data-[state=open]:fill-control-hover",
  "focus-visible:fill-control-hover"
);

/**
 * How many folders the working-folder menu offers.
 *
 * The recents list is longer than this on purpose - it is a history - but a
 * menu that grows with it turns picking a project into reading a list, and the
 * whole point of the chip is that it is faster than the settings pane. Three
 * covers what anyone is actually switching between; the folder picker below
 * them covers everything else.
 */
const MENU_FOLDERS = 3;

/**
 * The same recipe again for a round icon target, so the + at the head of the
 * row sits on the same fill as the pills after it rather than appearing out
 * of nowhere on hover. Ghost is right for an icon floating on a page; inside
 * a bar of filled pills it is the one hole in the row.
 *
 * The bang is on the open state alone: `IconButton` sets `bg-muted` there,
 * which is a real Tailwind background the class merger would happily keep
 * beside a custom `fill-*` utility.
 */
const ICON_PILL = cn(
  "fill-control text-foreground-secondary",
  "hover:fill-control-hover hover:text-foreground",
  "data-[state=open]:fill-control-hover! data-[state=open]:text-foreground"
);

export function Composer({ threadId, agentId, onSend, placeholder, streaming = false, className }) {
  const {
    agents,
    computers,
    threads,
    messages,
    setThreadMode,
    setThreadApproval,
    setThreadWorktree,
    setThreadTemporary,
    user,
    sendMessage,
    stopMessage,
    chatModels,
    defaultModelRef,
    chatPanelOpen,
    toggleChatPanel,
    openSettings,
    setView,
    saveAgent,
  } = useApp();
  const { chooseFolder, native: nativeWorkspace } = useWorkspace();
  const {
    folder: workspaceFolder,
    recents: recentFolders,
    remember: rememberFolder,
  } = useWorkingFolder();
  const { toast } = useToast();
  // Two handles on one editor. `inputRef` is its imperative API - focus, and
  // where the caret goes; `editorNodeRef` is the element itself, which the
  // palette needs in order to measure where the caret is on screen.
  const inputRef = React.useRef(null);
  const editorNodeRef = React.useRef(null);

  // The draft is stored per thread and survives navigation, a thread switch and
  // a relaunch. It used to be plain component state, so all three lost it.
  const [draft, setDraft, clearDraft] = useDraft(threadId);

  /**
   * The mode lives on the thread, not here.
   *
   * It decides which tools the agent is given, so it has to reach the request -
   * and it has to be the same next time this conversation is opened. As local
   * state it was neither: the picker moved, nothing downstream ever saw it, and
   * the value reset to Chat on every remount.
   */
  const thread = threads.find((t) => t.id === threadId);
  /**
   * Temporary lives on the thread now, not here.
   *
   * As component state it was a promise nothing kept: the dashed border said
   * the conversation would not be saved, and every message in it was mirrored
   * to the workspace folder like any other, because nothing outside this file
   * ever heard about it. On the thread it reaches the one place that decides
   * what is written down.
   */
  const temporary = Boolean(thread?.temporary);
  /**
   * Whether this conversation exists yet.
   *
   * Both counts, because they disagree for one frame and in one direction:
   * `messageCount` is the thread's own tally and a record written by an older
   * build may not carry it, while the messages themselves are the thing being
   * counted. Either one being non-zero means the conversation has started.
   */
  const started = Boolean(thread?.messageCount) || Boolean(messages?.[threadId]?.length);
  const mode = thread?.mode ?? DEFAULT_MODE;

  /**
   * What can be written as a pill in this box.
   *
   * Skills are fetched here rather than taken from the app store because they
   * are not in it - they are a workspace collection, read where they are
   * needed. The list is small and the hook caches it, so this costs a render
   * rather than a read per keystroke.
   */
  const { items: skills } = useCollection("skills");
  const mentions = React.useMemo(
    () =>
      mentionSpecs({
        // Agents are only something to mention in a room. In a one-agent
        // conversation `@` is a character, and offering a list of colleagues
        // who cannot answer would be offering something that does not work.
        agents: mode === "group" ? agents : [],
        skills: (skills ?? []).filter((one) => one.enabled !== false),
        // The app's own verbs, offered in the same list and under the same
        // slash. They are not sent to anybody - see `useCommands` - which is
        // why they are worth showing here rather than expecting to be
        // remembered.
        commands: COMMANDS,
        // The pill is plain DOM, so the face is prepared here where React
        // and the picture cache are: the icon rendered to markup, the picture
        // if it has already been read.
        face: (agent) => ({
          iconSrc: cachedAvatarSrc(agent),
          iconHtml: agent.icon ? renderToStaticMarkup(<Icon name={agent.icon} />) : null,
        }),
      }),
    [agents, skills, mode]
  );
  const palette = useMentionPalette({
    mentions,
    editorRef: inputRef,
    nodeRef: editorNodeRef,
    onChange: setDraft,
  });
  /**
   * The approval dial, beside the mode and separate from it. The mode is
   * what the turn holds; this is whether it asks. Lives on the thread too.
   */
  const approval = thread?.approval ?? DEFAULT_APPROVAL;
  const setApproval = (next) => {
    if (threadId) setThreadApproval(threadId, next);
  };
  // Before the first message the four rows behave as one choice: picking a
  // mode takes Temporary off, picking Temporary is picking it. After the
  // first message a mode is only a mode, and Temporary is no longer a row.
  const setMode = (next) => {
    if (!threadId) return;
    setThreadMode(threadId, next);
    if (!started) setThreadTemporary(threadId, false);
  };
  const chooseTemporary = () => {
    if (threadId && !started) setThreadTemporary(threadId, true);
  };

  /**
   * Dictation appends rather than replaces, because a person who typed half a
   * sentence and then reached for the microphone meant to finish it, not to
   * throw it away. The recording stops on its own once they stop talking.
   */
  const { settings: voiceSettings } = useVoiceSettings();
  const [voiceReady, setVoiceReady] = React.useState(false);

  // Whether a key is stored, so pressing the microphone with none can say so
  // instead of failing at the point of recording.
  React.useEffect(() => {
    let alive = true;
    voiceClient
      .configured()
      .then((value) => alive && setVoiceReady(Boolean(value)))
      .catch(() => alive && setVoiceReady(false));
    return () => {
      alive = false;
    };
  }, []);

  const dictation = useDictation({
    deviceId: voiceSettings.inputDeviceId,
    language: voiceSettings.language,
    onText: (text) => {
      setDraft((current) => `${current} ${text}`.trim());
      inputRef.current?.focus();
    },
    onError: (failure) =>
      toast({
        variant: "danger",
        title: "Dictation failed",
        description: failure.message,
      }),
  });

  const agent = agents.find((b) => b.id === agentId);

  // Configured providers win over the demo catalogue. Until one exists the
  // picker still has to show something, or the composer has a hole in it and
  // no explanation of why.
  const live = chatModels.length > 0;
  const fallback = live ? defaultModelRef || chatModels[0]?.ref : user.preferences.defaultModelId;
  const [modelId, setModelId] = React.useState(agent?.model ?? fallback);

  React.useEffect(() => {
    // An agent may name a model no configured provider offers - the provider
    // was removed, or the agent came from a workspace built elsewhere - so the
    // selection falls back rather than showing a model that cannot answer.
    const known = live
      ? chatModels.some((m) => m.ref === modelId)
      : MODELS.some((m) => m.id === modelId);
    if (!known) setModelId(agent?.model && live === false ? agent.model : fallback);
  }, [live, chatModels, modelId, fallback, agent?.model]);

  const computer = computers.find((c) => c.id === agent?.computerId);
  const ghost = temporary;

  /**
   * Files riding along with the next message.
   *
   * Held here rather than in the store, because until the message is sent they
   * are part of a draft nobody else has any business knowing about - and a
   * composer the user clears should take its attachments with it.
   */
  const [attachments, setAttachments] = React.useState([]);
  const fileInputRef = React.useRef(null);
  const [reading, setReading] = React.useState(false);
  const [dragging, setDragging] = React.useState(false);

  const addFiles = React.useCallback(
    async (files) => {
      const list = [...(files ?? [])];
      if (!list.length) return;
      setReading(true);
      try {
        const read = await Promise.all(list.map((file) => readAttachment(file)));
        const kept = read.filter((one) => one.ok).map((one) => one.attachment);
        for (const failure of read.filter((one) => !one.ok)) {
          toast({
            variant: "danger",
            title: failure.name,
            description: failure.why,
          });
        }
        if (kept.length) setAttachments((prev) => [...prev, ...kept]);
      } finally {
        setReading(false);
      }
    },
    [toast]
  );

  /**
   * A picture of the machine, attached without leaving the composer.
   *
   * The agent can take one whenever it likes; this is for the other direction -
   * "look at what is on the screen right now" - which otherwise means asking
   * the agent to look and hoping it looks at the right moment.
   */
  const takeScreenshot = React.useCallback(async () => {
    if (!computer) return;
    try {
      const shot = await machines.screenshot(computer.id);
      if (!shot) {
        toast({
          title: `${computer.name} has no display`,
          description: "Only a machine running a desktop can be photographed.",
        });
        return;
      }
      setAttachments((prev) => [
        ...prev,
        {
          id: `att-${Date.now().toString(36)}`,
          name: `${computer.name} screen.png`,
          kind: "image",
          size: Math.round((shot.length * 3) / 4),
          dataUrl: shot,
        },
      ]);
    } catch (error) {
      toast({
        variant: "danger",
        title: "Could not take a screenshot",
        description: error?.message,
      });
    }
  }, [computer, toast]);

  /** Routines this agent owns, which is what "run a routine" can mean here. */
  /**
   * The folder this agent's file tools work in.
   *
   * Stored on the agent rather than on the thread, because "which project" is a
   * standing fact about an agent and not a property of one conversation - and
   * because the session resolves it per turn, so a change here takes effect on
   * the very next message rather than on the next launch.
   *
   * A workspace-wide default sits behind it, which is what an agent uses until
   * someone points it somewhere.
   */
  const folder = React.useMemo(() => {
    const path = agent?.cwd || workspaceFolder || null;
    return {
      path,
      label: path ? basename(path) : "No folder",
    };
  }, [agent?.cwd, workspaceFolder]);

  /**
   * The folders the menu lists.
   *
   * The agent's own folder goes first rather than being left to `recents`,
   * because it is only in that list if it was picked from this menu. A folder
   * set on the agent anywhere else would fall off the end of a three-item list
   * and the menu would show nothing checked, which reads as no folder chosen.
   */
  const folderChoices = React.useMemo(() => {
    const list = [];
    for (const one of [agent?.cwd, ...recentFolders]) {
      if (one && !list.includes(one)) list.push(one);
    }
    return list.slice(0, MENU_FOLDERS);
  }, [agent?.cwd, recentFolders]);

  const setWorkingFolder = React.useCallback(
    (next) => {
      if (!agent) return;
      saveAgent(agent.id, { cwd: next || null });
      if (next) rememberFolder(next);
      toast({
        title: next
          ? `${agent.name} is working in ${basename(next)}`
          : "Back to the default folder",
        description: next ?? undefined,
      });
    },
    [agent, saveAgent, rememberFolder, toast]
  );

  const chooseWorkingFolder = React.useCallback(async () => {
    // Ask whether there is a disk behind this app, not whether the client
    // happens to expose a method. The adapter was missing `chooseFolder`
    // entirely, so this guard was answering "no desktop app" while running
    // inside the desktop app - the method being absent said nothing about the
    // platform, only that a passthrough had been forgotten.
    if (!nativeWorkspace) {
      toast({
        title: "Choosing a folder needs the desktop app",
        description: "A browser preview has no disk to point at.",
      });
      return;
    }
    try {
      const picked = await chooseFolder({
        title: `Where should ${agent?.name ?? "this agent"} work?`,
        startAt: folder.path ?? undefined,
        buttonLabel: "Work here",
      });
      // `null` is a cancel, which is not a failure and gets no message.
      if (picked) setWorkingFolder(picked);
    } catch (error) {
      toast({
        variant: "danger",
        title: "Could not open the folder picker",
        description: error?.message ?? String(error),
      });
    }
  }, [chooseFolder, nativeWorkspace, agent?.name, folder.path, setWorkingFolder, toast]);

  const empty = !draft.trim() && !attachments.length;

  /**
   * Sending into a turn that is already running.
   *
   * The composer used to refuse this outright, which made "no, not that file"
   * cost the whole turn: the only way to say it was to press stop and start
   * again. Now it goes to the running turn and is folded in at its next step,
   * so nothing already done is thrown away.
   *
   * Files are the exception. A turn in flight built its request from a history
   * that closed before they existed, so an attachment cannot join it - the
   * clip stays in the composer and travels with the next message rather than
   * disappearing into a turn that never saw it.
   */
  const steering = streaming && Boolean(draft.trim());

  // The model a command may have chosen. `/model haiku` moves the picker the
  // person can see, rather than quietly overriding it for one message.
  const runCommand = useCommands({ threadId, onModel: setModelId });

  const send = () => {
    const text = draft.trim();
    /**
     * `/compact`, `/clear`, `/help` - the box's own verbs.
     *
     * Checked before anything else, and before the "is there anything to
     * send" guard, because a command is not a message: it is answered here,
     * clears the box, and nothing crosses the bridge. An unrecognised slash
     * word is not a command at all and falls straight through to being sent,
     * which is what keeps `/usr/bin` and a skill mention working.
     */
    const handled = streaming ? false : runCommand(text);
    if (handled === true) {
      clearDraft();
      requestAnimationFrame(() => inputRef.current?.focus());
      return;
    }
    // A command that is a message - `/learn` - hands back what to send, and
    // it goes out from here so it carries this composer's model and mode
    // rather than whatever a call from inside the dispatcher would have
    // guessed.
    const outgoing = typeof handled === "string" ? handled : text;
    // A message that is only an image is a real message. "Look at this" is
    // implied by the act of attaching it, and refusing to send until someone
    // types a word is the kind of rule that makes a person type "hm".
    if (!text && !attachments.length) return;
    if (streaming && !text) return;
    clearDraft();
    if (!streaming) setAttachments([]);
    // The model travels with the message: a picker whose choice is dropped on
    // the way to the request is a control that lies about what it does.
    const options = {
      model: live ? modelId : undefined,
      attachments: streaming ? [] : attachments,
      // Which mode this was sent in. Read at send rather than at mount, so
      // changing the picker and pressing enter does what it looks like it does.
      mode,
    };
    if (onSend) onSend(outgoing, options);
    else if (threadId) sendMessage(threadId, outgoing, options);
    // the caret belongs back in the editor the instant the message leaves
    requestAnimationFrame(() => inputRef.current?.focus());
  };

  /**
   * Type an `@` or a `/` on the user's behalf and open the list under it.
   *
   * The menu entries are a signpost, not a second way of doing this: they put
   * the caret in the same place typing the character would, so somebody who
   * finds the feature through the menu once has learnt the keystroke for next
   * time. A space is inserted first when the caret is mid-word, because a
   * sigil that is not at a word boundary is not a sigil.
   */
  const insertSigil = (sigil) => {
    const at = inputRef.current?.getCaret?.() ?? draft.length;
    const before = draft.slice(0, at);
    const lead = before && !/\s$/.test(before) ? " " : "";
    const next = `${before}${lead}${sigil}${draft.slice(at)}`;
    const caret = before.length + lead.length + sigil.length;
    setDraft(next);
    // Next frame, not now. The menu that ran this hands focus back to its own
    // button when it closes, and that happens after this handler returns - so
    // a caret placed here was placed in an editor about to lose focus, and the
    // list opened over a button. The frame after, the menu is gone and the
    // editor is repainted with the sigil in it.
    requestAnimationFrame(() => {
      inputRef.current?.setCaret?.(caret);
      palette.track(next, caret);
    });
  };

  const onKeyDown = (e) => {
    // The palette is open and the caret is in a token: the arrows move through
    // the list, Enter picks rather than sends, and Escape closes the list
    // rather than stopping the turn.
    if (palette.onKeyDown(e)) {
      e.preventDefault();
      return;
    }
    if (e.key === "Escape") {
      // While a turn is running Escape is the stop button, because that is the
      // key people already reach for and the caret is almost always in here.
      // Only once there is nothing to stop does it fall back to letting go.
      if (streaming && threadId) {
        e.preventDefault();
        stopMessage(threadId);
        return;
      }
      e.currentTarget.blur();
      return;
    }
    if (e.key !== "Enter") return;
    const sendOnEnter = user.preferences.sendOnEnter !== false;
    const wantsSend = sendOnEnter ? !e.shiftKey : e.shiftKey || e.metaKey || e.ctrlKey;
    if (!wantsSend) return;
    e.preventDefault();
    send();
  };

  const modelOptions = React.useMemo(() => {
    if (live) {
      return chatModels.map((m) => ({
        value: m.ref,
        label: m.label,
        description: m.providerName,
      }));
    }
    return MODELS.map((m) => ({
      value: m.id,
      label: m.name,
      description: PROVIDERS.find((p) => p.id === m.providerId)?.name,
    }));
  }, [live, chatModels]);

  // ~4 characters per token is close enough to be useful and honest.
  const chars = draft.length;
  const tokens = Math.max(1, Math.round(chars / 4));

  /**
   * The glyphs that change meaning.
   *
   * Send becomes stop, the microphone becomes a square, the folder becomes a
   * branch - each is one control whose job has changed, and a glyph that is
   * simply different on the next frame reads as a glitch rather than a
   * change. Each pops in when it swaps and holds still on first render, so
   * opening a conversation mid-turn does not pop every icon in the row.
   */
  const sendGlyph = useSwap(streaming);
  const micGlyph = useSwap(dictation.state === "listening");
  const shieldGlyph = useSwap(approval === "auto");
  const folderGlyph = useSwap(Boolean(thread?.worktree));
  // And the line under the box, which says one of three things.
  const footnote = useSwap(steering ? "steering" : ghost ? "ghost" : "none");

  return (
    <div className={cn("w-full", className)}>
      <div
        data-mode={mode}
        className={cn(
          // The field surface, not a whisper of one. This was `fill-whisper`
          // - one and a half percent of the ink - which on a white ground is
          // nothing at all: the most important control in the app was a
          // rectangle you could only find by the hairline around it.
          // bg-composer, not bg-input: at this size the field is a surface, and
          // it takes the rail's colour so the two calm areas of the window are
          // one colour rather than two that nearly match.
          "relative flex w-full flex-col rounded-3xl bg-composer px-0.5 pt-0.5",
          // Everything in here sits on that surface, so the fill ladder inside
          // the composer is the field's rather than the ground's. Three
          // variables rather than a class on each control: the mode pill, the
          // approval pill, the model picker, the folder chip and every icon
          // button already ask for `fill-control`, `fill-nav` and `bg-muted`,
          // and this is what those three mean on a raised surface. Without it,
          // `--muted` in dark IS the field colour, which is why a pill that was
          // solid on white had nothing behind it on black.
          "[--fill-control:var(--fill-field)] [--fill-control-hover:var(--fill-field-hover)]",
          "[--fill-nav:var(--fill-field-hover)] [--muted:var(--fill-field-hover)]",
          "transition-colors duration-150 ease-out",
          // No edge, in any state. The hairline that used to fade in on hover
          // and focus is gone with every other stroke in the app; the fill is
          // the composer, and it is a real fill rather than the whisper it was
          // when the outline was carrying it.
          //
          // Temporary mode used to be a dashed border. It is now said by the
          // pill in the toolbar and by the line under the composer, which are
          // both words rather than a texture someone has to have been taught.
          // The whole composer lights up for a drag rather than a separate drop
          // zone appearing: the target is the thing you were already aiming at.
          dragging && "bg-input-hover"
        )}
        onDragOver={(e) => {
          if (!e.dataTransfer?.types?.includes("Files")) return;
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={(e) => {
          // Only when the pointer has actually left the composer, or crossing
          // the textarea inside it flickers the highlight on every mouse move.
          if (e.currentTarget.contains(e.relatedTarget)) return;
          setDragging(false);
        }}
        onDrop={(e) => {
          if (!e.dataTransfer?.files?.length) return;
          e.preventDefault();
          setDragging(false);
          addFiles(e.dataTransfer.files);
        }}
      >
        {/* Off screen rather than `display: none`, because a hidden input
            cannot be clicked programmatically in every browser. */}
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="sr-only"
          aria-hidden="true"
          tabIndex={-1}
          onChange={(e) => {
            addFiles(e.target.files);
            // Cleared, or picking the same file twice in a row fires nothing.
            e.target.value = "";
          }}
        />

        {/* The row folds open for the first attachment and away after the
            last, and every chip fades in on its own: a chip is only ever
            added while the person is looking, so there is no first render for
            it to be quiet on. */}
        <Collapse open={attachments.length > 0} innerClassName="flex flex-wrap gap-1.5 px-3 pt-2.5">
          {attachments.map((one) => (
            <span
              key={one.id}
              className="inline-flex h-7 max-w-56 items-center gap-1.5 rounded-full fill-control px-2.5 text-[11px] text-muted-foreground animate-fade-in"
            >
              {one.kind === "image" && one.dataUrl ? (
                <img
                  src={one.dataUrl}
                  alt=""
                  className="-ml-1.5 size-5 shrink-0 rounded-full object-cover"
                />
              ) : (
                <Paperclip className="size-3.5 shrink-0" aria-hidden="true" />
              )}
              <span className="min-w-0 truncate text-foreground">{one.name}</span>
              <span className="shrink-0 tabular-nums">{formatBytes(one.size)}</span>
              <button
                type="button"
                aria-label={`Remove ${one.name}`}
                onClick={() => setAttachments((prev) => prev.filter((x) => x.id !== one.id))}
                className="-mr-1 shrink-0 rounded-full p-0.5 text-muted-foreground/70 outline-none hover:text-foreground focus-visible:fill-control-hover"
              >
                <X className="size-3" aria-hidden="true" />
              </button>
            </span>
          ))}
          {reading ? (
            <span className="self-center text-[11px] text-muted-foreground animate-fade-in">
              Reading…
            </span>
          ) : null}
        </Collapse>

        <PromptEditor
          ref={inputRef}
          elementRef={editorNodeRef}
          value={draft}
          onChange={setDraft}
          mentions={mentions}
          onCaretChange={palette.track}
          // A screenshot on the clipboard is the most common attachment there
          // is, and pasting it is how everybody expects to attach it.
          onPasteFiles={addFiles}
          onKeyDown={onKeyDown}
          onBlur={palette.close}
          aria-label="Message"
          placeholder={
            placeholder ??
            (mode === "group"
              ? "Message the group… @ to pick who answers"
              : agent
                ? `Message ${agent.name}…`
                : "Send a message…")
          }
          className="px-4 py-3 text-base leading-relaxed sm:px-5 md:text-lg"
        />
        <MentionPalette palette={palette} agents={agents} />

        {/* The controls wrap rather than collide.
            Every chip on the left asks not to be shrunk - a mode pill reading
            "Accept ed..." is worse than no pill - so in a window narrow
            enough, or a conversation carrying enough chips, the row ran
            straight through the microphone and the send button and out of the
            composer. Wrapping is the one degradation that keeps every control
            its full size and still reachable, and `items-end` keeps send
            where it has always been: the bottom right corner. */}
        <div className="mx-0.5 mb-1 mt-1 flex items-end justify-between gap-2">
          <div className="flex min-w-0 flex-wrap items-center gap-1.5">
            <DropdownMenu
              side="top"
              align="start"
              // Ghost, like every other icon in this row. It was the emphasis
              // variant, which put a filled circle at one end of the bar and
              // the send button at the other - two competing round targets,
              // only one of which does anything on its own.
              // It rests with a fill rather than appearing on hover. Every
              // other control in this row is a visible target at rest, and the
              // one that adds a file to the message was the only thing here
              // you had to find by pointing at it.
              trigger={
                <IconButton size="md" label="Add to this message" className={ICON_PILL}>
                  <Plus />
                </IconButton>
              }
            >
              {/* Three groups, in the order a person reaches for them: what
                  goes WITH the message, what goes IN it, and what it runs on.
                  Rows carry a label and nothing else - the descriptions made
                  this a paragraph, and a menu is read in a glance or not at
                  all. Keyboard hints on the two that have one, because typing
                  the character is the faster way and this is where you learn
                  it. */}
              <MenuLabel>Attach</MenuLabel>
              <MenuItem icon={Paperclip} onSelect={() => fileInputRef.current?.click()}>
                File
              </MenuItem>
              <MenuItem
                icon={Camera}
                onSelect={takeScreenshot}
                disabled={!computer || computer.status !== "running"}
              >
                {computer ? `Screenshot of ${computer.name}` : "Screenshot"}
              </MenuItem>
              <MenuSeparator />
              <MenuLabel>Insert</MenuLabel>
              {mode === "group" ? (
                <MenuItem icon={AtSign} shortcut="@" onSelect={() => insertSigil("@")}>
                  Mention an agent
                </MenuItem>
              ) : null}
              <MenuItem icon={Sparkles} shortcut="/" onSelect={() => insertSigil("/")}>
                Use a skill
              </MenuItem>
              <MenuSeparator />
              <MenuLabel>Computer</MenuLabel>
              <MenuItem icon={Monitor} onSelect={() => setView("computers")}>
                {computer ? computer.name : "Choose a computer"}
              </MenuItem>
            </DropdownMenu>

            {/* the one divider the composer keeps: without it the + and the mode
                pill read as an undifferentiated run of round targets */}
            <span aria-hidden="true" className="mx-0.5 h-4 w-px shrink-0 bg-border/60" />

            <DropdownMenu
              side="top"
              align="start"
              trigger={({ ref, props }) => (
                <button ref={ref} type="button" {...props} className={PILL}>
                  <span className="font-medium">
                    {temporary
                      ? TEMPORARY.label
                      : (MODES.find((m) => m.value === mode)?.label ?? "Chat")}
                  </span>
                  <ChevronDown className="size-3.5 shrink-0 opacity-70" aria-hidden="true" />
                </button>
              )}
            >
              <MenuLabel>Mode</MenuLabel>
              {MODES.map((m) => (
                <MenuItem
                  key={m.value}
                  checked={mode === m.value && !(temporary && !started)}
                  description={m.hint}
                  onSelect={() => setMode(m.value)}
                >
                  {m.label}
                </MenuItem>
              ))}
              {started ? null : (
                <>
                  <MenuSeparator />
                  <MenuItem
                    checked={temporary}
                    icon={CalendarClock}
                    description={TEMPORARY.hint}
                    onSelect={chooseTemporary}
                  >
                    {TEMPORARY.label}
                  </MenuItem>
                </>
              )}
            </DropdownMenu>

            {/* How much this conversation asks. Amber once it is set to never
                ask, because a pill that looks the same whether or not anyone
                is checking the agent's work is the one thing here that must
                not blend in. */}
            <DropdownMenu
              side="top"
              align="start"
              trigger={({ ref, props }) => (
                <button
                  ref={ref}
                  type="button"
                  {...props}
                  aria-label={`Approval: ${APPROVALS.find((a) => a.id === approval)?.label ?? "Ask"}`}
                  className={cn(
                    PILL,
                    // The one sanctioned departure: never-ask wears amber. The
                    // bangs are because `fill-control` is a custom utility the
                    // class merger cannot see, so it has to be overruled in CSS
                    // rather than removed from the string.
                    approval === "auto" &&
                      "bg-warning/15! text-warning-ink hover:bg-warning/25! data-[state=open]:bg-warning/25!"
                  )}
                >
                  {approval === "auto" ? (
                    <ShieldAlert
                      key="alert"
                      onAnimationEnd={shieldGlyph.onAnimationEnd}
                      className={cn("size-3.5", shieldGlyph.swapping && "animate-pop-in")}
                      aria-hidden="true"
                    />
                  ) : (
                    <ShieldCheck
                      key="check"
                      onAnimationEnd={shieldGlyph.onAnimationEnd}
                      className={cn("size-3.5", shieldGlyph.swapping && "animate-pop-in")}
                      aria-hidden="true"
                    />
                  )}
                  <span className="font-medium">
                    {APPROVALS.find((a) => a.id === approval)?.label ?? "Ask"}
                  </span>
                  <ChevronDown className="size-3.5 shrink-0 opacity-70" aria-hidden="true" />
                </button>
              )}
            >
              <MenuLabel>Approval</MenuLabel>
              {APPROVALS.map((a) => (
                <MenuItem
                  key={a.id}
                  checked={approval === a.id}
                  description={a.hint}
                  onSelect={() => setApproval(a.id)}
                >
                  {a.label}
                </MenuItem>
              ))}
            </DropdownMenu>

            <Select
              size="xs"
              value={modelId}
              onChange={setModelId}
              options={modelOptions}
              ariaLabel="Model"
              // Same pill as the ones either side of it. The select brings its
              // own control fill and open state from `Select`; what it needs
              // here is the row's shape and the row's ink.
              className="h-[1.875rem] w-auto max-w-44 gap-1.5 rounded-full px-2.5 text-[11px] text-foreground-secondary"
            />

            {agent ? (
              // Which project this agent is working on.
              //
              // The file tools operate in one folder and ask before leaving it,
              // so which folder that is decides what the agent can see. That is
              // too important to live only in a settings pane three clicks
              // away, and it is the first thing to change when someone moves
              // from one project to another - so it sits next to the model.
              <DropdownMenu
                side="top"
                align="start"
                trigger={
                  <button
                    type="button"
                    title={
                      thread?.worktree
                        ? `Worktree at ${thread.worktree.path}`
                        : (folder.path ?? "No folder chosen")
                    }
                    className={PILL}
                  >
                    {thread?.worktree ? (
                      <GitBranch
                        key="branch"
                        onAnimationEnd={folderGlyph.onAnimationEnd}
                        className={cn(
                          "size-3.5 shrink-0",
                          folderGlyph.swapping && "animate-pop-in"
                        )}
                        aria-hidden="true"
                      />
                    ) : (
                      <FolderOpen
                        key="folder"
                        onAnimationEnd={folderGlyph.onAnimationEnd}
                        className={cn(
                          "size-3.5 shrink-0",
                          folderGlyph.swapping && "animate-pop-in"
                        )}
                        aria-hidden="true"
                      />
                    )}
                    <span className="min-w-0 truncate">
                      {thread?.worktree ? thread.worktree.branch : folder.label}
                    </span>
                  </button>
                }
              >
                <MenuLabel>Working folder</MenuLabel>
                {thread?.worktree ? (
                  // The conversation is in a worktree the agent entered. It
                  // outranks every folder below until it is left.
                  <MenuItem
                    icon={GitBranch}
                    checked
                    description={`A copy of ${basename(thread.worktree.root)} on branch ${thread.worktree.branch}. Leave to go back to the main checkout; the folder and branch stay.`}
                    onSelect={() => setThreadWorktree(threadId, null)}
                  >
                    Leave worktree {thread.worktree.name}
                  </MenuItem>
                ) : null}
                {folderChoices.map((one) => (
                  <MenuItem
                    key={one}
                    icon={Folder}
                    onSelect={() => setWorkingFolder(one)}
                    checked={one === folder.path}
                  >
                    {basename(one)}
                  </MenuItem>
                ))}
                <MenuItem icon={FolderOpen} onSelect={chooseWorkingFolder}>
                  Choose a folder…
                </MenuItem>
                {agent.cwd ? (
                  <MenuItem icon={FolderTree} onSelect={() => setWorkingFolder(null)}>
                    Use the workspace default
                  </MenuItem>
                ) : null}
              </DropdownMenu>
            ) : null}

            {computer ? (
              // The machine's name is the most natural handle on "show me what
              // it is working with", so the chip opens the thread panel rather
              // than sitting there as a label.
              <button
                type="button"
                title={computer.name}
                aria-pressed={chatPanelOpen}
                onClick={toggleChatPanel}
                className={cn(
                  PILL,
                  "max-w-40",
                  // A toggle, not a picker: pressed has to read as held down
                  // rather than as merely hovered, so it keeps the secondary
                  // fill and the full-strength ink.
                  chatPanelOpen && "fill-secondary text-foreground hover:fill-secondary-hover"
                )}
              >
                <Monitor className="size-3.5 shrink-0" aria-hidden="true" />
                <span className="min-w-0 truncate">{computer.name}</span>
              </button>
            ) : null}
          </div>

          <div className="flex shrink-0 items-center gap-1">
            {/* Always here. Hiding it until voice was configured meant adding
                the key changed nothing on screen, and the one affordance that
                would have told someone voice exists was the thing being hidden
                from them. Unconfigured, it explains itself and offers the fix. */}
            <IconButton
              size="md"
              label={
                dictation.state === "listening"
                  ? "Stop dictating"
                  : dictation.state === "thinking"
                    ? "Transcribing"
                    : "Dictate"
              }
              disabled={dictation.state === "thinking"}
              className={cn(dictation.state === "listening" && "text-destructive-ink")}
              onClick={() => {
                if (!voiceReady) {
                  toast({
                    title: "Voice needs an ElevenLabs key",
                    description:
                      "Add one under Settings, Secrets and the microphone starts working.",
                    action: {
                      label: "Open Secrets",
                      onClick: () => openSettings("secrets"),
                    },
                  });
                  return;
                }
                if (dictation.state === "listening") dictation.stop();
                else dictation.start();
              }}
            >
              {dictation.state === "listening" ? (
                <Square
                  key="stop"
                  onAnimationEnd={micGlyph.onAnimationEnd}
                  className={cn(micGlyph.swapping && "animate-pop-in")}
                />
              ) : (
                <Mic
                  key="mic"
                  onAnimationEnd={micGlyph.onAnimationEnd}
                  className={cn(micGlyph.swapping && "animate-pop-in")}
                />
              )}
            </IconButton>
            {/* Two buttons while a turn is running, never one wearing two
                hats. Stop is always where it was, so nobody has to empty the
                box to find it; send only appears once there is something to
                say, and says in its label that it goes to the turn in progress
                rather than starting a new one. */}
            {steering ? (
              <button
                type="button"
                onClick={send}
                title={`Send to ${agent?.name ?? "the agent"} now - it arrives at the next step`}
                aria-label="Send to the running turn"
                className={cn(
                  "flex h-[1.875rem] shrink-0 items-center gap-1.5 rounded-full px-3",
                  "fill-secondary text-[13px] text-foreground outline-none",
                  "transition-colors duration-150 ease-out hover:fill-control-hover",
                  "focus-visible:fill-control-hover",
                  // Only ever appears in answer to typing, so it is always new.
                  "animate-fade-in"
                )}
              >
                <SendArrow className="size-3.5" />
                Send now
              </button>
            ) : null}
            <button
              type="button"
              onClick={streaming ? () => threadId && stopMessage(threadId) : send}
              disabled={!streaming && empty}
              aria-label={streaming ? "Stop generating" : "Send message"}
              className={cn(
                "flex size-[1.875rem] shrink-0 items-center justify-center rounded-full",
                "outline-none transition-colors duration-150 ease-out",
                // Empty is a different control, not a faded one. Half-opacity
                // on a solid dark circle is a grey blob - the loudest thing in
                // the composer, saying nothing - so an empty composer gets a
                // quiet surface and a quiet arrow, and the solid button
                // appears when there is something to send.
                empty && !streaming
                  ? "fill-secondary text-icon-muted focus-visible:fill-control-hover"
                  : "bg-foreground text-background hover:opacity-90 focus-visible:opacity-90",
                "disabled:pointer-events-none"
              )}
            >
              {streaming ? (
                <Square
                  key="stop"
                  onAnimationEnd={sendGlyph.onAnimationEnd}
                  className={cn("size-3 fill-current", sendGlyph.swapping && "animate-pop-in")}
                />
              ) : (
                <SendArrow
                  key="send"
                  onAnimationEnd={sendGlyph.onAnimationEnd}
                  className={cn("size-3.5", sendGlyph.swapping && "animate-pop-in")}
                />
              )}
            </button>
          </div>
        </div>
      </div>

      <div className="mt-1 flex h-4 items-center justify-between px-3">
        {steering ? (
          // Said before they press it, not after. "It arrives at the next
          // step" is the one thing about this that is not obvious: the turn is
          // in the middle of a tool call and the message waits for it to
          // finish, so a few seconds of nothing happening is the design.
          <span
            key="steering"
            onAnimationEnd={footnote.onAnimationEnd}
            className={cn(
              "text-[10px] text-muted-foreground",
              footnote.swapping && "animate-fade-in"
            )}
          >
            {agent?.name ?? "The agent"} is working - this reaches it at the next step.
          </span>
        ) : ghost ? (
          <span
            key="ghost"
            onAnimationEnd={footnote.onAnimationEnd}
            className={cn(
              "text-[10px] text-muted-foreground",
              footnote.swapping && "animate-fade-in"
            )}
          >
            Temporary chat - this won&rsquo;t be saved.
          </span>
        ) : (
          <span key="none" />
        )}
        {chars > 80 ? (
          <span className="text-[10px] tabular-nums text-muted-foreground">
            {chars} characters · ~{tokens} tokens
          </span>
        ) : null}
      </div>
    </div>
  );
}
