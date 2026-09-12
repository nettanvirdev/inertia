import * as React from "react";
import { Check, ChevronDown, Monitor, Moon, RotateCcw, Sun } from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { useTheme } from "@/lib/theme";
import {
  ACCENTS,
  DARK_PALETTES,
  LIGHT_PALETTES,
  APPEARANCE_DEFAULTS,
  CONTRASTS,
  DENSITIES,
  FONT_FAMILIES,
  LINE_HEIGHTS,
  MESSAGE_WIDTHS,
  MONO_FAMILIES,
  RADII,
  TRANSCRIPTS,
  appearanceOf,
  isBehindMore,
  shownFirst,
} from "@/lib/appearance";
import { Avatar } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { Segmented } from "@/components/ui/segmented";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toast";
import { SettingsRow, SettingsSection } from "../SettingsRow";

/**
 * A 3-element mock of the shell - titlebar, rail, message column.
 *
 * Each half wears the theme class it is previewing and then paints itself from
 * ordinary tokens, so it shows the real palette rather than a copy. It used to
 * be a copy: six hex values under a comment asking whoever retuned globals.css
 * to retune these as well. Nobody did, and the picker spent a release offering
 * a theme the app no longer had - which is what happens when a comment is given
 * a stylesheet's job.
 *
 * Nothing in here may use a `dark:` variant. That variant matches any
 * descendant of `.dark`, so in a dark app it would fire inside the LIGHT tile
 * too. The bare tokens are already theme-aware, which is the point of them.
 */
function ShellMock({ theme, split }) {
  // "System" is the only tile that shows two halves - one per resolved theme.
  const panes = split ? ["light", "dark"] : [theme];
  return (
    <div className="flex h-[4.5rem] w-full overflow-hidden rounded-md xl:h-[6.5rem]">
      {panes.map((mode) => (
        <div
          key={mode}
          className={cn(mode, "bg-background flex h-full flex-1 flex-col")}
        >
          <div className="bg-sidebar h-2 w-full shrink-0" />
          <div className="flex min-h-0 flex-1">
            <div className="bg-sidebar h-full w-3 shrink-0 p-1">
              <div className="bg-foreground/35 h-1 w-full rounded-full" />
            </div>
            <div className="flex min-w-0 flex-1 flex-col justify-end gap-1 p-1.5">
              <div className="bg-muted ml-auto h-2.5 w-2/3 rounded-[3px]" />
              <div className="bg-foreground/50 h-1 w-5/6 rounded-full" />
              <div className="bg-foreground/30 h-1 w-3/5 rounded-full" />
              <div className="bg-card-lighter h-2.5 w-full rounded-[4px]" />
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

const THEME_TILES = [
  { value: "system", label: "System", icon: Monitor, split: true },
  { value: "light", label: "Light", icon: Sun, theme: "light" },
  { value: "dark", label: "Dark", icon: Moon, theme: "dark" },
];

/**
 * The shared tile chrome.
 *
 * Theme, accent and typeface are all "pick one of these, and the tile shows you
 * what you would get". Sharing the frame is what stops them reading as three
 * unrelated widgets stacked on one screen.
 */
function Tile({ selected, onClick, ariaLabel, className, children }) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      aria-label={ariaLabel}
      onClick={onClick}
      className={cn(
        "flex flex-col gap-1.5 rounded-lg p-1.5 text-left outline-none",
        "transition-colors duration-150 ease-out",
        "focus-visible:fill-control-hover",
        selected ? "bg-muted" : "fill-control hover:fill-control-hover",
        className,
      )}
    >
      {children}
    </button>
  );
}

/**
 * An accent swatch.
 *
 * It paints the colour for the RESOLVED theme, not the picked one: each accent
 * ships a light and a dark value, and showing the light chip while the app is
 * dark would promise a colour the user is never going to see. "Ink" gets the
 * live foreground, because the absence of an accent is still a swatch you have
 * to be able to choose.
 */
function AccentSwatch({ accent, isDark, selected, onClick }) {
  const color = accent.light
    ? isDark
      ? accent.dark
      : accent.light
    : "var(--foreground)";
  const on = accent.on
    ? isDark
      ? accent.on.dark
      : accent.on.light
    : "var(--background)";
  return (
    <Tile
      selected={selected}
      onClick={onClick}
      ariaLabel={accent.label}
      className="gap-1"
    >
      <span
        aria-hidden="true"
        className="flex h-8 w-full items-center justify-center rounded-md"
        style={{ background: color, color: on }}
      >
        {selected ? <Check className="size-3.5" /> : null}
      </span>
      <span className="truncate px-0.5 pb-0.5 text-[0.6875rem] text-foreground/90">
        {accent.label}
      </span>
    </Tile>
  );
}

/**
 * A palette tile, painted in its own colours.
 *
 * The swatch is a miniature of what the choice actually does: the ground, a
 * surface sitting on it, and the ink on top. A row of names would make somebody
 * pick four times to find out what "Mist" means, and the difference between two
 * grounds is exactly the thing a word cannot carry.
 *
 * It is painted from the `swatch` on the entry rather than by rendering the
 * real tokens in a `.light` / `.dark` subtree. A subtree would be more truthful
 * and it would also be wrong here, because the tile has to show the palette you
 * have NOT selected - all four at once, in a window that is only ever in one of
 * them.
 */
function PaletteTile({ palette, selected, onClick }) {
  return (
    <Tile selected={selected} onClick={onClick} ariaLabel={palette.label} className="gap-1">
      <span
        aria-hidden="true"
        className="flex h-10 w-full items-center gap-1.5 overflow-hidden rounded-md px-2"
        style={{ background: palette.swatch.ground, color: palette.swatch.ink }}
      >
        {/* the surface, with a line of "text" on it: a ground alone tells you
            nothing about whether a card will be found on it */}
        <span
          className="flex h-6 flex-1 items-center rounded-sm px-1.5"
          style={{ background: palette.swatch.surface }}
        >
          <span
            className="h-1 w-full rounded-full"
            style={{ background: palette.swatch.ink, opacity: 0.45 }}
          />
        </span>
        {selected ? <Check className="size-3.5 shrink-0" /> : null}
      </span>
      <span className="truncate px-0.5 pb-0.5 text-[0.6875rem] text-foreground/90">
        {palette.label}
      </span>
    </Tile>
  );
}

/**
 * A grid of choices that does not open at full height.
 *
 * The colours doubled, and a picker that shows all of them at once stops being
 * a choice and becomes a paint chart: the person who wanted the app to be blue
 * reads past nine swatches to find out that "Ink" means no colour at all. So
 * the first row is what the app has always offered and the rest is one click
 * away - and the click is a real disclosure, with a count on it, not a chevron
 * that could mean anything.
 *
 * It opens already expanded when the current choice lives behind the fold.
 * Anything else would be a picker that cannot show you what you have chosen,
 * which is the one thing a picker has to do.
 */
function ExpandingGrid({ label, options, value, columns, children }) {
  const hidden = React.useMemo(() => options.length - shownFirst(options).length, [options]);
  const [open, setOpen] = React.useState(() => isBehindMore(options, value));

  // A choice made elsewhere - reset, a synced preference - must not leave the
  // selected swatch folded away where nobody can see it.
  React.useEffect(() => {
    if (isBehindMore(options, value)) setOpen(true);
  }, [options, value]);

  const shown = open ? options : shownFirst(options);

  return (
    <>
      <div role="radiogroup" aria-label={label} className={columns}>
        {shown.map(children)}
      </div>
      {hidden > 0 && !open ? (
        <button
          type="button"
          onClick={() => setOpen(true)}
          className="mt-2 flex items-center gap-1 rounded-md px-0.5 py-1 text-[0.6875rem] text-muted-foreground transition-colors hover:text-foreground"
        >
          <ChevronDown className="size-3" aria-hidden="true" />
          {hidden} more
        </button>
      ) : null}
    </>
  );
}

export function AppearancePane() {
  const { user, setPreference, setPreferences } = useApp();
  const { theme, setTheme, resolved } = useTheme();
  const { toast } = useToast();

  const a = appearanceOf(user.preferences);
  const isDark = resolved === "dark";

  const reset = () => {
    setPreferences({ ...APPEARANCE_DEFAULTS });
    toast({ variant: "success", title: "Appearance reset to defaults" });
  };

  return (
    <div className="w-full">
      <SettingsSection flat title="Theme">
        <div
          role="radiogroup"
          aria-label="Theme"
          className="grid grid-cols-3 gap-2"
        >
          {THEME_TILES.map((tile) => {
            const selected = theme === tile.value;
            const Icon = tile.icon;
            return (
              <Tile
                key={tile.value}
                selected={selected}
                onClick={() => setTheme(tile.value)}
                ariaLabel={tile.label}
              >
                <ShellMock theme={tile.theme} split={tile.split} />
                <span className="flex items-center gap-1 px-0.5 pb-0.5 text-xs text-foreground/90">
                  <Icon
                    className="size-3.5 shrink-0 text-muted-foreground"
                    aria-hidden="true"
                  />
                  <span className="truncate">{tile.label}</span>
                  {selected ? (
                    <Check
                      className="ml-auto size-3.5 shrink-0 text-foreground"
                      aria-hidden="true"
                    />
                  ) : null}
                </span>
              </Tile>
            );
          })}
        </div>
      </SettingsSection>

      <SettingsSection flat
        title="Accent"
        description="Used for the primary button and whatever is currently selected. Ink is the original: no colour at all."
      >
        <ExpandingGrid
          label="Accent"
          options={ACCENTS}
          value={a.accent}
          columns="grid grid-cols-4 gap-2 sm:grid-cols-7"
        >
          {(accent) => (
            <AccentSwatch
              key={accent.value}
              accent={accent}
              isDark={isDark}
              selected={a.accent === accent.value}
              onClick={() => setPreference("accent", accent.value)}
            />
          )}
        </ExpandingGrid>
      </SettingsSection>

      {/* Two grounds rather than one, because a person who reads on cream in
          daylight still wants near-black at night. The pane shows both at once
          instead of only the theme that happens to be on, so switching theme
          never reveals a palette nobody chose. */}
      <SettingsSection flat
        title="Light ground"
        description="The paper the app is drawn on in the light theme. Only the surfaces move - status colours, code highlighting and your accent are the same in every one."
      >
        <ExpandingGrid
          label="Light ground"
          options={LIGHT_PALETTES}
          value={a.lightPalette}
          columns="grid grid-cols-4 gap-2"
        >
          {(palette) => (
            <PaletteTile
              key={palette.value}
              palette={palette}
              selected={a.lightPalette === palette.value}
              onClick={() => setPreference("lightPalette", palette.value)}
            />
          )}
        </ExpandingGrid>
        <p className="mt-1.5 px-0.5 text-[0.6875rem] leading-relaxed text-muted-foreground">
          {LIGHT_PALETTES.find((p) => p.value === a.lightPalette)?.hint}
        </p>
      </SettingsSection>

      <SettingsSection flat
        title="Dark ground"
        description="The same choice for the dark theme."
      >
        <ExpandingGrid
          label="Dark ground"
          options={DARK_PALETTES}
          value={a.darkPalette}
          columns="grid grid-cols-4 gap-2"
        >
          {(palette) => (
            <PaletteTile
              key={palette.value}
              palette={palette}
              selected={a.darkPalette === palette.value}
              onClick={() => setPreference("darkPalette", palette.value)}
            />
          )}
        </ExpandingGrid>
        <p className="mt-1.5 px-0.5 text-[0.6875rem] leading-relaxed text-muted-foreground">
          {DARK_PALETTES.find((p) => p.value === a.darkPalette)?.hint}
        </p>
      </SettingsSection>

      <SettingsSection flat
        title="Typeface"
        description="Mostly the faces this computer already has. Inter and Source Serif are bundled, as is a Bengali partner for every one of them - so বাংলা is drawn deliberately rather than by whatever the platform happens to find."
      >
        <div
          role="radiogroup"
          aria-label="Typeface"
          className="grid grid-cols-2 gap-2 sm:grid-cols-4"
        >
          {FONT_FAMILIES.map((font) => {
            const selected = a.fontFamily === font.value;
            return (
              <Tile
                key={font.value}
                selected={selected}
                onClick={() => setPreference("fontFamily", font.value)}
                ariaLabel={font.label}
              >
                {/* the sample is set in the font it names, so the choice is
                    legible BEFORE it is made rather than after */}
                <span
                  aria-hidden="true"
                  className="flex h-10 w-full items-center justify-center gap-1.5 rounded-md card-surface-subtle text-lg text-foreground"
                  style={{ fontFamily: font.stack }}
                >
                  {/* Both scripts, because both are what the stack decides. A
                      Latin-only sample let a face be chosen that drew বাংলা in
                      something else entirely, which is the bug this is for. */}
                  Ag<span className="text-muted-foreground">অআ</span>
                </span>
                <span className="flex min-w-0 flex-col px-0.5 pb-0.5">
                  <span className="flex items-center gap-1 text-[0.6875rem] text-foreground/90">
                    <span className="truncate">{font.label}</span>
                    {selected ? (
                      <Check
                        className="ml-auto size-3 shrink-0 text-foreground"
                        aria-hidden="true"
                      />
                    ) : null}
                  </span>
                  <span className="truncate text-[0.625rem] text-muted-foreground">
                    {font.description}
                  </span>
                </span>
              </Tile>
            );
          })}
        </div>
      </SettingsSection>

      <SettingsSection flat
        title="Monospace"
        description="Used for code blocks, file paths and anything that has to line up in a column. The sample is the test that matters: a zero you cannot mistake for an O."
      >
        <div
          role="radiogroup"
          aria-label="Monospace typeface"
          className="grid grid-cols-2 gap-2 sm:grid-cols-4"
        >
          {MONO_FAMILIES.map((font) => {
            const selected = a.monoFamily === font.value;
            return (
              <Tile
                key={font.value}
                selected={selected}
                onClick={() => setPreference("monoFamily", font.value)}
                ariaLabel={font.label}
              >
                <span
                  aria-hidden="true"
                  className="flex h-10 w-full items-center justify-center rounded-md card-surface-subtle text-base text-foreground"
                  style={{ fontFamily: font.stack }}
                >
                  0O1l
                </span>
                <span className="flex min-w-0 flex-col px-0.5 pb-0.5">
                  <span className="flex items-center gap-1 text-[0.6875rem] text-foreground/90">
                    <span className="truncate">{font.label}</span>
                    {selected ? (
                      <Check
                        className="ml-auto size-3 shrink-0 text-foreground"
                        aria-hidden="true"
                      />
                    ) : null}
                  </span>
                  <span className="truncate text-[0.625rem] text-muted-foreground">
                    {font.description}
                  </span>
                </span>
              </Tile>
            );
          })}
        </div>
      </SettingsSection>

      <SettingsSection title="Shape and layout">
        <SettingsRow
          label="Corner radius"
          description="One token, so every card, button and field rounds together."
          control={
            <div className="flex items-center gap-2">
              {/* three chips on the live token: they restyle themselves the
                  instant the choice lands, which is the whole proof */}
              <span aria-hidden="true" className="flex items-center gap-1">
                <span className="size-5 rounded-[var(--radius)] bg-muted" />
                <span className="size-5 rounded-[var(--radius)] bg-foreground/20" />
              </span>
              <Segmented
                size="xs"
                label="Corner radius"
                value={a.radius}
                onChange={(v) => setPreference("radius", v)}
                options={RADII.map((r) => ({ value: r.value, label: r.label }))}
              />
            </div>
          }
        />
        <SettingsRow
          label="Interface density"
          description="Compact tightens every gutter and row height in the app at once."
          control={
            <Segmented
              size="xs"
              label="Interface density"
              value={a.density}
              onChange={(v) => setPreference("density", v)}
              options={DENSITIES.map((d) => ({
                value: d.value,
                label: d.label,
              }))}
            />
          }
        />
        <SettingsRow
          label="Sidebar on launch"
          description="What the rail does at startup. It can still be toggled at any time."
          control={
            <Segmented
              size="xs"
              label="Sidebar on launch"
              value={a.sidebarDefault}
              onChange={(v) => setPreference("sidebarDefault", v)}
              options={[
                { value: "expanded", label: "Expanded" },
                { value: "collapsed", label: "Collapsed" },
              ]}
            />
          }
        />
        <SettingsRow
          label="Secondary text contrast"
          description="How far the muted greys - timestamps, descriptions, hints - sit from the text you are reading. High is for a dim panel or a bright room."
          control={
            <Segmented
              size="xs"
              label="Secondary text contrast"
              value={a.contrast}
              onChange={(v) => setPreference("contrast", v)}
              options={CONTRASTS.map((c) => ({
                value: c.value,
                label: c.label,
              }))}
            />
          }
        />
        <SettingsRow
          label="Reduce motion"
          htmlFor="set-reduce-motion"
          description="Nothing slides, grows or folds: menus, panels and sections fade in briefly instead. Colour changes and spinners stay, so nothing appears without warning."
          control={
            <Switch
              id="set-reduce-motion"
              size="sm"
              label="Reduce motion"
              checked={!!a.reduceMotion}
              onCheckedChange={(v) => setPreference("reduceMotion", v)}
            />
          }
        />
      </SettingsSection>

      <SettingsSection
        title="Size"
        description="Zoom scales the whole window, including the parts that are not text. Message size only moves the transcript."
      >
        <SettingsRow
          label="Interface zoom"
          description="Every pixel of the window - text, icons and padding together - the way a browser zoom works."
          control={
            <>
              <Slider
                className="w-40"
                label="Interface zoom"
                min={70}
                max={150}
                step={5}
                value={a.zoom}
                formatValue={(v) => `${v} percent`}
                onChange={(v) => setPreference("zoom", v)}
              />
              <span className="w-10 shrink-0 text-right text-[0.6875rem] tabular-nums text-muted-foreground">
                {a.zoom}%
              </span>
            </>
          }
        />
        <SettingsRow
          label="Message font size"
          description="The size a reply is set at. The transcript under Preview moves with it."
          control={
            <>
              <Slider
                className="w-40"
                label="Message font size"
                min={12}
                max={20}
                step={1}
                value={a.fontSize}
                formatValue={(v) => `${v} pixels`}
                onChange={(v) => setPreference("fontSize", v)}
              />
              <span className="w-10 shrink-0 text-right text-[0.6875rem] tabular-nums text-muted-foreground">
                {a.fontSize}px
              </span>
            </>
          }
        />
        <SettingsRow
          label="Code font size"
          description="Code blocks, file paths and anything that has to line up in a column."
          control={
            <>
              <Slider
                className="w-40"
                label="Code font size"
                min={11}
                max={18}
                step={0.5}
                value={a.codeSize}
                formatValue={(v) => `${v} pixels`}
                onChange={(v) => setPreference("codeSize", v)}
              />
              <span className="w-10 shrink-0 text-right text-[0.6875rem] tabular-nums text-muted-foreground">
                {a.codeSize}px
              </span>
            </>
          }
        />
        <SettingsRow
          label="Tool card height"
          description="How tall an opened tool card grows before it scrolls inside itself, so a long file does not push the conversation off the screen."
          control={
            <>
              <Slider
                className="w-40"
                label="Tool card height"
                min={160}
                max={800}
                step={40}
                value={a.toolHeight}
                formatValue={(v) => `${v} pixels`}
                onChange={(v) => setPreference("toolHeight", v)}
              />
              <span className="w-10 shrink-0 text-right text-[0.6875rem] tabular-nums text-muted-foreground">
                {a.toolHeight}px
              </span>
            </>
          }
        />
        <SettingsRow
          label="Message width"
          description="How far a line of a reply is allowed to run before it wraps."
          control={
            <Segmented
              size="xs"
              label="Message width"
              value={a.messageWidth}
              onChange={(v) => setPreference("messageWidth", v)}
              options={MESSAGE_WIDTHS.map((w) => ({
                value: w.value,
                label: w.label,
              }))}
            />
          }
        />
      </SettingsSection>

      <SettingsSection title="Transcript">
        <SettingsRow
          label="Message style"
          description="Bubbles tint what you wrote so a glance tells you who said what. Flat drops the tint and the thread reads as one document."
          control={
            <Segmented
              size="xs"
              label="Message style"
              value={a.transcript}
              onChange={(v) => setPreference("transcript", v)}
              options={TRANSCRIPTS.map((t) => ({
                value: t.value,
                label: t.label,
              }))}
            />
          }
        />
        <SettingsRow
          label="Line spacing"
          description="How much air is left between the lines of a reply."
          control={
            <Segmented
              size="xs"
              label="Line spacing"
              value={a.lineHeight}
              onChange={(v) => setPreference("lineHeight", v)}
              options={LINE_HEIGHTS.map((l) => ({
                value: l.value,
                label: l.label,
              }))}
            />
          }
        />
        <SettingsRow
          label="Show timestamps"
          htmlFor="set-timestamps"
          description="A relative time under every message instead of nothing at all."
          control={
            <Switch
              id="set-timestamps"
              size="sm"
              label="Show timestamps"
              checked={!!a.showTimestamps}
              onCheckedChange={(v) => setPreference("showTimestamps", v)}
            />
          }
        />
        <SettingsRow
          label="Show avatars in transcript"
          htmlFor="set-avatars"
          description="Off gives a tighter column with names only."
          control={
            <Switch
              id="set-avatars"
              size="sm"
              label="Show avatars in transcript"
              checked={!!a.showAvatars}
              onCheckedChange={(v) => setPreference("showAvatars", v)}
            />
          }
        />
      </SettingsSection>

      <SettingsSection flat title="Preview">
        {/* Everything in here is on the live tokens rather than on props, which
            is the same path the real transcript takes. If the preview does not
            move, the app would not have moved either. */}
        <div
          className={cn(
            "flex flex-col rounded-lg card-surface-subtle",
            a.density === "compact" ? "gap-2 p-2.5" : "gap-3 p-3.5",
          )}
        >
          {/* justify-end rather than ml-auto: the bubble's own width is the
              measure now, and an auto margin on an element that is already as
              wide as its cap does nothing. */}
          <div className="flex justify-end">
            <div className="chat-measure rounded-2xl rounded-br-sm bg-user-message-background px-3 py-2">
              <p className="chat-text leading-relaxed text-foreground">
                Summarise yesterday&apos;s incident in three lines.
              </p>
              <Collapse open={Boolean(a.showTimestamps)}>
                <p className="mt-1 text-[0.6875rem] text-muted-foreground">
                  2 minutes ago
                </p>
              </Collapse>
            </div>
          </div>
          <div className="flex gap-2">
            {a.showAvatars ? (
              <Avatar name="Atlas" size="sm" className="mt-0.5 animate-pop-in" />
            ) : null}
            <div className="min-w-0 flex-1">
              <p className="text-xs font-medium text-foreground">Atlas</p>
              <p className="chat-text chat-measure mt-0.5 leading-relaxed text-foreground">
                The API gateway dropped 4% of requests for eleven minutes after
                a bad config rollout. Traffic recovered on rollback. A guard is
                now in place.
              </p>
              {/* the one place code font size shows itself; it used to sit
                  beside its slider, which made that row the odd one out */}
              <pre className="mt-2 overflow-x-auto rounded-lg card-surface-raised px-3 py-2">
                <code className="font-mono text-foreground">
                  git rebase --onto main 0O1l~1
                </code>
              </pre>
              <Collapse open={Boolean(a.showTimestamps)}>
                <p className="mt-1 text-[0.6875rem] text-muted-foreground">
                  Just now
                </p>
              </Collapse>
            </div>
          </div>
          {/* the accent has exactly two jobs in this app, and both are here */}
          <div className="flex items-center gap-2">
            <Button variant="primary" size="xs">
              Approve
            </Button>
            <Button variant="subtle" size="xs">
              Decline
            </Button>
            <span className="accent-ink ml-auto text-[0.6875rem] font-medium">
              Selected
            </span>
          </div>
        </div>
      </SettingsSection>

      <SettingsSection title="Reset">
        <SettingsRow
          label="Reset appearance"
          description="Puts theme colour, typeface, shape, size and transcript options back to the shipped values. Nothing else is touched."
          control={
            <Button variant="subtle" size="xs" onClick={reset}>
              <RotateCcw />
              Reset appearance
            </Button>
          }
        />
      </SettingsSection>
    </div>
  );
}
