// Shown once after each update. Newest first; keep entries short and in the
// user's words. Release notes on GitHub are generated from commits.
export interface Release {
  version: string;
  highlights: string[];
}

export const RELEASES: Release[] = [
  {
    version: "2.0.0",
    highlights: [
      "Rebuilt from scratch: logins are faster and use less memory.",
      "Passwords, PINs and TOTP secrets are now encrypted, with the key kept in your computer's keychain, instead of sitting in a plain file.",
      "Schedule a daily login — 8:45 AM is recommended — and AutoLogin runs from the tray.",
      "Add accounts by pasting from the Cirrus dashboard.",
      "Move to a new computer with a password-protected backup.",
      "A screenshot is saved when a login fails, so you can see what went wrong.",
      "Your AutoLogin 1.x accounts were brought over automatically.",
    ],
  },
];
