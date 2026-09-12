/**
 * Put text on the clipboard, and say honestly whether it landed.
 *
 * Every copy control in the window used to call `navigator.clipboard.writeText`
 * without waiting for it and then announce success. In a packaged window that
 * announcement was a lie: the permission handler denies everything the app has
 * no use for, `clipboard-sanitized-write` was on that list, and the promise
 * rejected into nothing while a green toast said "Copied to clipboard".
 *
 * The permission is granted now, for the app's own page only. This still keeps
 * the older path underneath, because the async clipboard also rejects when the
 * document is not focused - which happens when the click came from a control
 * inside a dialog that just took focus back, and which `execCommand` does not
 * care about. It runs in the same task as the click, so the gesture is intact.
 */
export async function copyText(value) {
  const text = String(value ?? "");
  if (!text) return false;
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return fallbackCopy(text);
  }
}

/** The pre-async-clipboard way: a hidden textarea, selected, and `copy`. */
export function fallbackCopy(text) {
  if (typeof document === "undefined" || !document.body) return false;
  const area = document.createElement("textarea");
  area.value = text;
  // Off-screen rather than hidden: `display:none` cannot hold a selection.
  area.setAttribute("readonly", "");
  area.style.cssText = "position:fixed;top:0;left:-9999px;opacity:0";
  document.body.appendChild(area);
  const previous = document.activeElement;
  try {
    area.select();
    area.setSelectionRange(0, text.length);
    return document.execCommand?.("copy") === true;
  } catch {
    return false;
  } finally {
    area.remove();
    if (previous && typeof previous.focus === "function") previous.focus();
  }
}

/**
 * What is on the clipboard, for the one place that needs to read it.
 *
 * Asks the main process rather than the page. Reading the clipboard is a
 * permission this app denies - see the handler behind `readClipboard` - so
 * `navigator.clipboard.readText()` is the path that quietly returns nothing in
 * a packaged build, which is exactly how a paste that does nothing gets
 * shipped.
 */
export async function readClipboardText() {
  const bridge = typeof window !== "undefined" ? window.electronAPI : null;
  if (bridge?.readClipboard) {
    try {
      return String((await bridge.readClipboard()) ?? "");
    } catch {
      return "";
    }
  }
  try {
    return String((await navigator.clipboard.readText()) ?? "");
  } catch {
    return "";
  }
}
