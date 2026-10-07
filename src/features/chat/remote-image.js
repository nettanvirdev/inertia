/**
 * Pictures from the internet, which are only fetched when the person asks.
 *
 * A picture in a reply is an address the model wrote, and fetching it is a
 * request from this machine to that host - with the reader's IP, at the moment
 * they scroll past, and with whatever the model put in the query string. That
 * makes an image in a reply a way to send data out, whoever planted the
 * instruction to write it. So the transcript draws the host and waits for a
 * click; the click is the person deciding to make that request.
 *
 * Remembered for the session per address, so a picture loaded once is not
 * asked about again every time its message scrolls back into view.
 */

const LOADED = new Set();

/** The host a remote picture would be fetched from, or null for any other picture. */
export function remoteHost(src) {
  const value = String(src ?? "");
  if (!/^(https?:)?\/\//i.test(value)) return null;
  try {
    return new URL(value, "https://placeholder.invalid").host || null;
  } catch {
    return null;
  }
}

/** The person asked for this one. */
export function allowRemote(src) {
  LOADED.add(String(src ?? ""));
}

/** Whether a picture may be fetched without asking: anything not remote, or a remote one the person already loaded. */
export function mayFetch(src) {
  return !remoteHost(src) || LOADED.has(String(src ?? ""));
}
