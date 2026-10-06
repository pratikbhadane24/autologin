import { describe, expect, test } from "vitest";
import { DEVICE, forPlatform, IS_MOBILE } from "./platform";

describe("platform copy", () => {
  test("detects a desktop browser as not mobile", () => {
    expect(IS_MOBILE).toBe(false);
    expect(DEVICE).toBe("computer");
  });

  test("keeps plain lines and picks the desktop variant", () => {
    expect(forPlatform("Same everywhere")).toBe("Same everywhere");
    expect(forPlatform({ desktop: "Runs from the tray", mobile: null })).toBe("Runs from the tray");
    expect(forPlatform({ desktop: null, mobile: "Phone only" })).toBeNull();
  });
});
