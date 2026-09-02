import { describe, expect, it } from "vitest";
import i18n from "./i18n";

describe("i18n", () => {
  it("opens in Simplified Chinese by default", () => {
    expect(i18n.resolvedLanguage).toBe("zh-CN");
  });
});
