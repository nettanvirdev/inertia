import * as React from "react";
import { AudioLines, Check, Mic, Play, RotateCcw, Search, Square } from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { voice as voiceClient, isDesktop } from "@/lib/voice";
import { speaker } from "@/lib/speaker";
import { useVoiceSettings, useAudioDevices } from "@/hooks/use-voice-settings";
import { useDictation } from "@/hooks/use-dictation";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { EmptyState } from "@/components/ui/empty-state";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { Select } from "@/components/ui/select";
import { SkeletonRow } from "@/components/ui/skeleton";
import { Slider } from "@/components/ui/slider";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toast";
import { SettingsCard, SettingsRow, SettingsSection } from "../SettingsRow";

/**
 * Voice mode, connected to something.
 *
 * This pane was a mock, and an oddly specific one: it offered "MacBook Pro
 * Microphone" and "AirPods Pro" on every machine including the Windows ones,
 * listed three invented voices, and its Play button ran a timer to the length
 * of a sample it never played. Every control here now reaches ElevenLabs or the
 * operating system. The voices are the ones the account actually has, the
 * devices are the ones plugged in, the preview synthesises a real sentence and
 * plays it, and the microphone test records and transcribes for real.
 *
 * The one thing that is deliberately local is the speaking rate. It is applied
 * at playback by the Web Audio node rather than sent to the API, so dragging
 * the slider costs nothing and does not spend a character of the quota
 * re-rendering a sentence that has already been synthesised.
 */

/** Short enough to be cheap, long enough to hear what a voice sounds like. */
const SAMPLE = "Here is what I sound like. I will read replies in this voice.";

/** Past this the list is a wall, and a filter earns its row. */
const FILTER_AT = 8;

const NUMBER = new Intl.NumberFormat();

function VoiceRow({ voice, selected, onSelect, playing, busy, onToggle }) {
  return (
    <div
      className={cn(
        "flex items-center gap-2 rounded-lg px-2 py-1.5 transition-colors duration-150 ease-out",
        selected ? "bg-muted" : "hover:fill-nav"
      )}
    >
      <button
        type="button"
        role="radio"
        aria-checked={selected}
        onClick={() => onSelect(voice.id)}
        className="flex min-w-0 flex-1 items-center gap-2 rounded-lg text-left outline-none focus-visible:fill-nav"
      >
        <span className="flex size-4 shrink-0 items-center justify-center">
          {selected ? <Check className="size-3.5 text-foreground" aria-hidden="true" /> : null}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-xs text-foreground/90">{voice.name}</span>
          <span className="mt-0.5 block truncate text-[0.6875rem] leading-relaxed text-muted-foreground">
            {busy ? "Synthesising a sample…" : playing ? "Playing…" : voice.tone || voice.category}
          </span>
        </span>
      </button>
      <IconButton
        size="sm"
        label={playing ? `Stop the ${voice.name} sample` : `Hear ${voice.name}`}
        disabled={busy}
        onClick={() => onToggle(voice.id)}
      >
        {busy ? <Spinner size="sm" /> : playing ? <Square /> : <Play />}
      </IconButton>
    </div>
  );
}

export function VoicePane() {
  const { openSettings } = useApp();
  const { toast } = useToast();
  const { settings, update, loading: loadingSettings } = useVoiceSettings();
  const { inputs, outputs } = useAudioDevices();

  const [ready, setReady] = React.useState(null); // a key is stored
  const [working, setWorking] = React.useState(false); // and ElevenLabs accepted it
  const [account, setAccount] = React.useState(null);
  const [voices, setVoices] = React.useState([]);
  const [models, setModels] = React.useState([]);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState(null);
  const [filter, setFilter] = React.useState("");

  const [preview, setPreview] = React.useState(null); // { id, phase: "loading" | "playing" }
  const [heard, setHeard] = React.useState(null);

  // The shared speaker, so previewing a voice stops a reply that is being read
  // aloud in a thread behind this dialog rather than playing over it.
  React.useEffect(() => () => speaker.stop(), []);

  const load = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    setWorking(false);
    try {
      const configured = await voiceClient.configured();
      setReady(configured);
      if (!configured) {
        setVoices([]);
        setModels([]);
        setAccount(null);
        return;
      }
      // In parallel: three independent reads, and waiting for them in turn is
      // three round trips of blank pane for no reason.
      const [list, catalogue, who] = await Promise.all([
        voiceClient.voices(),
        voiceClient.models(),
        voiceClient.test().catch(() => null),
      ]);
      setVoices(list);
      setModels(catalogue);
      setAccount(who);
      setWorking(true);
    } catch (failure) {
      // The key is still stored; ElevenLabs simply would not take it. Keeping
      // those apart is what lets the pane say "Rejected" rather than "Not
      // connected", which would send someone off to find a key they already have.
      setError(failure.message);
      setVoices([]);
      setModels([]);
      setAccount(null);
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    load();
  }, [load]);

  // A stored voice that the account no longer has would leave the pane showing
  // nothing selected and every reply failing. Fall back to the first real one.
  React.useEffect(() => {
    if (loading || !voices.length) return;
    if (!settings.voiceId || !voices.some((entry) => entry.id === settings.voiceId)) {
      update({ voiceId: voices[0].id });
    }
  }, [loading, voices, settings.voiceId, update]);

  const shown = React.useMemo(() => {
    const needle = filter.trim().toLowerCase();
    if (!needle) return voices;
    return voices.filter((entry) =>
      `${entry.name} ${entry.tone} ${entry.category}`.toLowerCase().includes(needle)
    );
  }, [voices, filter]);

  async function togglePreview(id) {
    if (preview?.id === id) {
      speaker.stop();
      setPreview(null);
      return;
    }
    speaker.stop();
    setPreview({ id, phase: "loading" });
    try {
      await speaker.speak([SAMPLE], {
        voiceId: id,
        modelId: settings.modelId,
        rate: settings.rate,
        onUtterance: () => setPreview({ id, phase: "playing" }),
      });
    } catch (failure) {
      toast({
        variant: "danger",
        title: "Could not play that voice",
        description: failure.message,
      });
    } finally {
      setPreview((current) => (current?.id === id ? null : current));
    }
  }

  /* -- the microphone test ------------------------------------------------ */
  const dictation = useDictation({
    deviceId: settings.inputDeviceId,
    language: settings.language,
    onText: (text) => setHeard(text),
    onError: (failure) =>
      toast({
        variant: "danger",
        title: "Dictation failed",
        description: failure.message,
      }),
  });

  if (!isDesktop()) {
    return (
      <EmptyState
        icon={AudioLines}
        title="Voice needs the desktop app"
        description="A browser tab has no backend to reach ElevenLabs from, and no microphone permission worth granting a page that renders model output."
      />
    );
  }

  /**
   * There is no "enable voice mode" switch any more.
   *
   * There were two controls saying one thing: a key either is stored or is
   * not, and a switch that claimed to turn the feature on while no key existed
   * turned nothing on. Worse, it hid the microphone from the composer, so the
   * only affordance that would have told someone voice exists was the thing
   * being withheld until they had already found the settings pane. Storing a
   * key is the opt in; removing it is the opt out. What is left below is the
   * one genuine choice, which is whether replies are read to you unasked.
   */
  return (
    <div className="w-full">
      <SettingsSection title="Voice mode">
        <SettingsRow
          label="Read replies aloud"
          htmlFor="set-voice-autospeak"
          description="Speaks each reply as it finishes. Code blocks are named rather than read out, and you can stop it at any point."
          control={
            <Switch
              id="set-voice-autospeak"
              size="sm"
              label="Read replies aloud"
              checked={Boolean(settings.autoSpeak)}
              disabled={loadingSettings || !working}
              onCheckedChange={(value) => update({ autoSpeak: value })}
            />
          }
        />
      </SettingsSection>

      <div className="mt-5">
        <SettingsSection
          flat
          title="ElevenLabs"
          description="Speech and transcription both run through ElevenLabs, using the key stored under Secrets. Nothing is recorded or sent until you press the microphone."
        >
          <SettingsCard className="flex flex-wrap items-center gap-2 p-2">
            <span className="flex size-7 shrink-0 items-center justify-center rounded-full fill-control text-muted-foreground">
              <AudioLines className="size-3.5" aria-hidden="true" />
            </span>
            <div className="min-w-0 flex-1">
              <p className="truncate text-xs text-foreground/90">ElevenLabs</p>
              <p className="truncate text-[0.6875rem] leading-relaxed text-muted-foreground">
                {loading
                  ? "Checking…"
                  : error
                    ? error
                    : ready && account
                      ? `On the ${account.tier} plan, with ${NUMBER.format(
                          account.remaining
                        )} of ${NUMBER.format(account.limit)} characters left.`
                      : ready
                        ? "A key is stored."
                        : "No key is stored, so nothing here can run."}
              </p>
            </div>
            {/* A stored key and a working key are different states, and the
                badge has to say which. Reporting "Connected" over a line that
                says the key was rejected is the pane lying to itself. */}
            <Badge variant={error ? "danger" : working ? "success" : "neutral"} size="sm" dot>
              {loading ? "Checking" : error ? "Rejected" : working ? "Connected" : "Not connected"}
            </Badge>
            {ready ? (
              <Button size="xs" variant="subtle" onClick={load} disabled={loading}>
                {loading ? <Spinner size="sm" /> : <RotateCcw />}
                Refresh
              </Button>
            ) : (
              <Button size="xs" variant="primary" onClick={() => openSettings("secrets")}>
                Add the key
              </Button>
            )}
          </SettingsCard>
        </SettingsSection>

        <SettingsSection
          flat
          title="Voice"
          description="The voices on your ElevenLabs account. Add more in their voice library and they appear here."
        >
          {voices.length > FILTER_AT ? (
            <Input
              size="xs"
              leadingIcon={<Search />}
              placeholder="Filter voices"
              aria-label="Filter voices"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
            />
          ) : null}

          {loading ? (
            <SkeletonRow lines={4} />
          ) : !ready ? (
            <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
              Add the key above and the account&apos;s voices are listed here.
            </p>
          ) : error ? (
            // An empty list and a list that could not be fetched look identical
            // on screen, and telling someone their account has no voices when
            // the key was rejected sends them to the wrong place entirely.
            <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
              The voices could not be read, so there is nothing to choose from yet.
            </p>
          ) : shown.length ? (
            <SettingsCard
              role="radiogroup"
              aria-label="Voice"
              className="flex animate-fade-in flex-col gap-0.5 p-1.5"
            >
              {shown.map((entry) => (
                <VoiceRow
                  key={entry.id}
                  voice={entry}
                  selected={entry.id === settings.voiceId}
                  onSelect={(id) => update({ voiceId: id })}
                  busy={preview?.id === entry.id && preview.phase === "loading"}
                  playing={preview?.id === entry.id && preview.phase === "playing"}
                  onToggle={togglePreview}
                />
              ))}
            </SettingsCard>
          ) : (
            <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
              {filter ? "No voice matches that." : "This account has no voices yet."}
            </p>
          )}
        </SettingsSection>

        <SettingsSection title="Delivery">
          <SettingsRow
            label="Model"
            description="Flash is the cheapest and starts speaking soonest. The larger models sound better and cost more per character."
            control={
              <Select
                size="xs"
                className="w-56"
                ariaLabel="Speech model"
                value={settings.modelId}
                onChange={(value) => update({ modelId: value })}
                options={
                  models.length
                    ? models.map((model) => ({
                        value: model.id,
                        label: model.name,
                        description: model.description,
                      }))
                    : [{ value: settings.modelId, label: settings.modelId }]
                }
              />
            }
          />

          <SettingsRow
            label="Speaking rate"
            description="Applied as it plays, so changing it never re-synthesises anything."
            control={
              <>
                <Slider
                  className="w-40"
                  label="Speaking rate"
                  min={0.5}
                  max={2}
                  step={0.05}
                  value={Number(settings.rate) || 1}
                  onChange={(value) => update({ rate: value })}
                  formatValue={(v) => `${v.toFixed(2)} times normal speed`}
                />
                <span className="w-10 shrink-0 text-right text-[0.6875rem] tabular-nums text-muted-foreground">
                  {Number(settings.rate).toFixed(2)}×
                </span>
              </>
            }
          />
        </SettingsSection>

        <SettingsSection
          title="Devices"
          description="Names appear once you have allowed the microphone. Until then the list is real but unlabelled, which is the browser protecting you from a page that would otherwise learn your hardware."
        >
          <SettingsRow
            label="Microphone"
            control={
              <Select
                size="xs"
                className="w-56"
                ariaLabel="Microphone"
                value={settings.inputDeviceId}
                onChange={(value) => update({ inputDeviceId: value })}
                options={[{ value: "default", label: "System default" }, ...inputs]}
              />
            }
          />
          <SettingsRow
            label="Output"
            description="Playback follows the system output. Choosing a device here is remembered for when Inertia can route to one."
            control={
              <Select
                size="xs"
                className="w-56"
                ariaLabel="Output device"
                value={settings.outputDeviceId}
                onChange={(value) => update({ outputDeviceId: value })}
                options={[{ value: "default", label: "System default" }, ...outputs]}
              />
            }
          />

          <div className="flex flex-col gap-2.5 px-4 py-3">
            <div className="flex flex-wrap items-center gap-2">
              <Button
                size="xs"
                variant={dictation.state === "listening" ? "danger" : "subtle"}
                disabled={!ready || dictation.state === "thinking"}
                onClick={() =>
                  dictation.state === "listening" ? dictation.stop() : dictation.start()
                }
              >
                {dictation.state === "thinking" ? <Spinner size="sm" /> : <Mic />}
                {dictation.state === "listening"
                  ? "Stop and transcribe"
                  : dictation.state === "thinking"
                    ? "Transcribing…"
                    : "Test the microphone"}
              </Button>
              <p className="min-w-0 flex-1 text-[0.6875rem] leading-relaxed text-muted-foreground">
                Say a sentence. It stops on its own once you go quiet.
              </p>
            </div>

            <Collapse open={dictation.state === "listening"}>
              <Progress value={dictation.level} max={1} label="Microphone level" />
            </Collapse>

            <Collapse open={Boolean(heard)}>
              <p className="rounded-lg fill-control px-2.5 py-2 text-[0.6875rem] leading-relaxed text-foreground">
                {heard}
              </p>
            </Collapse>
          </div>
        </SettingsSection>
      </div>
    </div>
  );
}
