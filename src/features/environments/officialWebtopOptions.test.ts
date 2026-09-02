import { describe, expect, it } from "vitest";
import { officialWebtopOptions } from "./officialWebtopOptions";

describe("official Webtop options", () => {
  it("contains every selectable option once", () => {
    const keys = officialWebtopOptions.map((option) => option.key);
    expect(new Set(keys).size).toBe(keys.length);
    expect(keys).toHaveLength(85);
  });

  it("does not expose app-managed identities or secrets", () => {
    const keys = new Set(officialWebtopOptions.map((option) => option.key));
    for (const managed of ["PUID", "PGID", "TZ", "LC_ALL", "CUSTOM_USER", "PASSWORD", "SELKIES_MASTER_TOKEN"]) {
      expect(keys.has(managed)).toBe(false);
    }
  });
});
