// Which kind of device the app runs on, for copy that differs between
// desktop and phones. The WebView's user agent is known synchronously.
export const IS_MOBILE = /Android|iPhone|iPad/i.test(navigator.userAgent);

/** "computer" or "phone", for sentences like "stays on this computer". */
export const DEVICE = IS_MOBILE ? "phone" : "computer";

/** A line that differs by platform; `null` means it doesn't apply there. */
export type PlatformText = string | { desktop: string | null; mobile: string | null };

export function forPlatform(text: PlatformText): string | null {
  if (typeof text === "string") return text;
  return IS_MOBILE ? text.mobile : text.desktop;
}
