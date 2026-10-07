import * as React from "react";
import { PREFERENCE_DEFAULTS } from "@/lib/appearance";
import { useApp } from "@/lib/store";

/**
 * Dates and numbers, written the way this user reads them.
 *
 * The app had a "Language" picker that changed nothing, because there is no
 * translation layer to change. What a desktop app can honestly offer instead is
 * the half of a locale that needs no translated strings: the order of a date,
 * the shape of a clock, the separator in a thousand. That is what lives here.
 *
 * Two preferences drive everything - `locale` and `timezone` - and both use the
 * literal string "system" to mean "ask the computer". Storing the resolved
 * value instead would freeze a choice made on a laptop that has since flown to
 * another continent, which is exactly the case a time zone setting exists for.
 *
 * Every formatter is total. `Intl` throws a RangeError on a tag or a zone it
 * does not know - a preference written by an older build, or a workspace folder
 * carried to a machine with a smaller ICU - and a timestamp is never worth
 * taking a screen down for, so an unusable choice falls back to the system's
 * own formatting rather than propagating.
 */

/** The value both preferences use for "whatever this computer says". */
export const SYSTEM = "system";

/**
 * The locales offered. English-speaking regions first because the interface
 * text is English regardless, then the formats people most often ask for.
 * A locale here changes dates and numbers only - see the pane's copy.
 */
export const LOCALES = [
  { value: SYSTEM, label: "Match system" },
  { value: "en-US", label: "English (United States)" },
  { value: "en-GB", label: "English (United Kingdom)" },
  { value: "en-CA", label: "English (Canada)" },
  { value: "en-AU", label: "English (Australia)" },
  { value: "de-DE", label: "German (Germany)" },
  { value: "fr-FR", label: "French (France)" },
  { value: "es-ES", label: "Spanish (Spain)" },
  { value: "pt-BR", label: "Portuguese (Brazil)" },
  { value: "nl-NL", label: "Dutch (Netherlands)" },
  { value: "sv-SE", label: "Swedish (Sweden)" },
  { value: "ja-JP", label: "Japanese (Japan)" },
  { value: "zh-CN", label: "Chinese (Simplified)" },
];

/**
 * A short list, not a complete one. A full IANA database is around six hundred
 * zones and a dropdown of six hundred rows is a search problem, not a setting;
 * these cover the offsets a single-user desktop app is realistically read in,
 * and "Match system" covers everyone else correctly by definition.
 */
export const TIME_ZONES = [
  { value: SYSTEM, label: "Match system" },
  { value: "UTC", label: "UTC" },
  { value: "Pacific/Auckland", label: "Auckland" },
  { value: "Australia/Sydney", label: "Sydney" },
  { value: "Asia/Tokyo", label: "Tokyo" },
  { value: "Asia/Singapore", label: "Singapore" },
  { value: "Asia/Kolkata", label: "Kolkata" },
  { value: "Asia/Dubai", label: "Dubai" },
  { value: "Europe/Berlin", label: "Berlin" },
  { value: "Europe/London", label: "London" },
  { value: "Africa/Lagos", label: "Lagos" },
  { value: "America/Sao_Paulo", label: "Sao Paulo" },
  { value: "America/New_York", label: "New York" },
  { value: "America/Chicago", label: "Chicago" },
  { value: "America/Denver", label: "Denver" },
  { value: "America/Los_Angeles", label: "Los Angeles" },
  { value: "Pacific/Honolulu", label: "Honolulu" },
];

/** What the computer itself would use, for the "Match system" rows. */
export function systemLocale() {
  try {
    return new Intl.DateTimeFormat().resolvedOptions().locale;
  } catch {
    return "en-US";
  }
}

export function systemTimeZone() {
  try {
    return new Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

export function resolveLocale(preference) {
  return preference && preference !== SYSTEM ? preference : systemLocale();
}

export function resolveTimeZone(preference) {
  return preference && preference !== SYSTEM ? preference : systemTimeZone();
}

/**
 * A formatter that always exists.
 *
 * Two fallbacks rather than one: a bad zone can be dropped while keeping the
 * user's locale, and only a locale the runtime cannot parse at all falls all
 * the way through to the platform default.
 */
function formatter(locale, timeZone, options) {
  try {
    return new Intl.DateTimeFormat(locale, { ...options, timeZone });
  } catch {
    /* fall through */
  }
  try {
    return new Intl.DateTimeFormat(locale, options);
  } catch {
    return new Intl.DateTimeFormat(undefined, options);
  }
}

function toDate(value) {
  if (!value) return null;
  const date = value instanceof Date ? value : new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

function format(value, preferences, options) {
  const date = toDate(value);
  if (!date) return "-";
  const locale = resolveLocale(preferences?.locale);
  const timeZone = resolveTimeZone(preferences?.timezone);
  return formatter(locale, timeZone, options).format(date);
}

/** "2 Sep 2026" - the shape a settings row or a file listing wants. */
export function formatDate(value, preferences) {
  return format(value, preferences, { year: "numeric", month: "short", day: "numeric" });
}

/** "2 Sep 2026, 16:20" - a date plus a clock, in the chosen zone. */
export function formatDateTime(value, preferences) {
  return format(value, preferences, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Just the clock, for a row whose day is already stated above it. */
export function formatTime(value, preferences) {
  return format(value, preferences, { hour: "2-digit", minute: "2-digit" });
}

/** "GMT+2" for the zone right now. Computed rather than written into the list,
 *  because half these zones change offset twice a year. */
export function zoneOffsetLabel(zone, at = new Date()) {
  try {
    const parts = new Intl.DateTimeFormat("en-US", {
      timeZone: zone === SYSTEM ? systemTimeZone() : zone,
      timeZoneName: "shortOffset",
    }).formatToParts(at);
    return parts.find((p) => p.type === "timeZoneName")?.value ?? "";
  } catch {
    return "";
  }
}

/** Numbers follow the same locale, so 1,234.5 and 1.234,5 do not both appear. */
export function formatNumber(value, preferences, options) {
  if (value == null || Number.isNaN(Number(value))) return "-";
  try {
    return new Intl.NumberFormat(resolveLocale(preferences?.locale), options).format(Number(value));
  } catch {
    return String(value);
  }
}

/**
 * The hook every screen uses.
 *
 * It hands back bound formatters rather than the two raw strings, so no call
 * site has to remember to pass the preferences through - the bug that let the
 * old settings pane ship a time zone nothing read.
 */
export function useDateFormat() {
  const { user } = useApp();
  const locale = user?.preferences?.locale ?? PREFERENCE_DEFAULTS.locale;
  const timezone = user?.preferences?.timezone ?? PREFERENCE_DEFAULTS.timezone;

  return React.useMemo(() => {
    const preferences = { locale, timezone };
    return {
      locale,
      timezone,
      resolvedLocale: resolveLocale(locale),
      resolvedTimeZone: resolveTimeZone(timezone),
      formatDate: (value) => formatDate(value, preferences),
      formatDateTime: (value) => formatDateTime(value, preferences),
      formatTime: (value) => formatTime(value, preferences),
      formatNumber: (value, options) => formatNumber(value, preferences, options),
    };
  }, [locale, timezone]);
}

/**
 * The UTC day a timestamp falls in, as `YYYY-MM-DD`, or `null` when there is no
 * day to name.
 *
 * Every screen that groups records by day used to reach for `iso.slice(0, 10)`,
 * which is correct right up until a record has no timestamp - and records come
 * off a folder the user is invited to hand-edit, so that is a case rather than
 * an impossibility. `null` is the answer those callers need: it groups, it
 * sorts, and it is not a string that pretends to be a date.
 *
 * Deliberately not locale-aware. This is a grouping key, not a label - the label
 * is derived from it afterwards, and a key that shifted with the time zone
 * preference would reshuffle the groups without changing the data.
 */
export function utcDayKey(value) {
  const date = toDate(value);
  return date ? date.toISOString().slice(0, 10) : null;
}
