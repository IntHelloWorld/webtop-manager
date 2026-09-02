import { describe, expect, it } from "vitest";
import { en } from "./en";
import { zhCN } from "./zh-CN";

function keys(value: unknown, prefix = ""): string[] {
  if (typeof value !== "object" || value === null) return [prefix];
  return Object.entries(value).flatMap(([key, nested]) => keys(nested, prefix ? `${prefix}.${key}` : key));
}

describe("translations", () => {
  it("keeps English and Simplified Chinese keys in sync", () => {
    expect(keys(zhCN).sort()).toEqual(keys(en).sort());
  });
});
