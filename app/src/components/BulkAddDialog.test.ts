import { describe, expect, test } from "vitest";
import { summary } from "./BulkAddDialog";

describe("bulk add summary", () => {
  test("names new and refreshed accounts and what still needs setup", () => {
    expect(summary(3, 2, 1)).toBe("Added 3 accounts, updated 2 accounts, 1 needs setup");
    expect(summary(1, 0, 0)).toBe("Added 1 account");
    expect(summary(0, 2, 2)).toBe("Updated 2 accounts, 2 need setup");
    expect(summary(0, 0, 0)).toBe("Nothing changed");
  });
});
