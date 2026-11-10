/**
 * @ai-generated deepseek-v4-flash
 * 自动主题亮度判定测试：阈值边界与灰区保持行为。
 */
import { describe, expect, it, vi } from "vitest";

// resolveAutoTheme 所在模块有运行时依赖，这里只测纯判定，替换掉副作用模块
vi.mock("./settings", () => ({
  getSettings: () => ({ theme: "dark" }),
  getEffectiveTheme: () => "dark",
  setEffectiveTheme: () => {},
}));
vi.mock("./system", () => ({
  sampleBackdropLuminance: async () => null,
}));

import {
  AUTO_THEME_DARK_THRESHOLD,
  AUTO_THEME_LIGHT_THRESHOLD,
  resolveAutoTheme,
} from "./auto-theme";

describe("resolveAutoTheme", () => {
  it("should switch to dark when the backdrop is bright", () => {
    const theme = resolveAutoTheme(0.9, "light");

    expect(theme).toBe("dark");
  });

  it("should switch to light when the backdrop is dark", () => {
    const theme = resolveAutoTheme(0.1, "dark");

    expect(theme).toBe("light");
  });

  it("should keep the current theme in the gray zone", () => {
    const midpoint = (AUTO_THEME_DARK_THRESHOLD + AUTO_THEME_LIGHT_THRESHOLD) / 2;

    expect(resolveAutoTheme(midpoint, "dark")).toBe("dark");
    expect(resolveAutoTheme(midpoint, "light")).toBe("light");
  });

  it("should treat the thresholds as inclusive", () => {
    expect(resolveAutoTheme(AUTO_THEME_DARK_THRESHOLD, "light")).toBe("dark");
    expect(resolveAutoTheme(AUTO_THEME_LIGHT_THRESHOLD, "dark")).toBe("light");
  });
});
