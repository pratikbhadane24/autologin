// Shown once after each update. Newest first; keep entries short and in the
// user's words. Release notes on GitHub are generated from commits.
import type { PlatformText } from "../lib/platform";

export interface Release {
  version: string;
  highlights: PlatformText[];
}

export const RELEASES: Release[] = [
  {
    version: "2.0.0",
    highlights: [
      "Rebuilt from scratch: logins are faster and use less memory.",
      {
        desktop:
          "Passwords, PINs and TOTP secrets are now encrypted, with the key kept in your computer's keychain, instead of sitting in a plain file.",
        mobile:
          "Passwords, PINs and TOTP secrets are encrypted, with the key protected by this phone's Android Keystore.",
      },
      {
        desktop: "Schedule a daily login — 8:45 AM is recommended — and AutoLogin runs from the tray.",
        mobile:
          "Schedule a daily login — 8:45 AM is recommended. A notification starts it with one tap, or AutoLogin can log in by itself.",
      },
      "Dhan, 5Paisa and Tradejini accounts log in too.",
      "Motilal Oswal's new login needs your API Secret: add it to each Motilal account.",
      "Add accounts by sending them from Cirrus, or by pasting.",
      "Move to a new computer or phone with a password-protected backup.",
      "A screenshot is saved when a login fails, so you can see what went wrong.",
      {
        desktop: "Your AutoLogin 1.x accounts were brought over automatically.",
        mobile: null,
      },
    ],
  },
];
