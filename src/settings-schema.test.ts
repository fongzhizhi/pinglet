/**
 * @ai-generated deepseek-v4-flash
 * 设置解析与归一化的纯逻辑测试（node 环境，不依赖 DOM / localStorage）。
 */
import { describe, expect, it } from "vitest";
import {
  SHORTCUT_MAX_LENGTH,
  createDefaultSettings,
  normalizeShortcut,
  parseSettings,
} from "./settings-schema";

describe("parseSettings", () => {
  it("should return defaults when storage is empty", () => {
    const settings = parseSettings(null);

    expect(settings).toEqual(createDefaultSettings());
  });

  it("should default to auto theme, close-to-tray and enabled notifications", () => {
    const settings = createDefaultSettings();

    expect(settings.theme).toBe("auto");
    expect(settings.closeToTray).toBe(true);
    expect(settings.systemNotifications).toBe(true);
    expect(settings.doNotDisturb).toBe(false);
  });

  it("should default panel effects to enabled", () => {
    const settings = createDefaultSettings();

    expect(settings.panelEffects).toEqual({ alertPulse: true, workingFlow: true });
  });

  it("should keep explicit opt-outs from storage", () => {
    const settings = parseSettings(
      JSON.stringify({
        theme: "light",
        closeToTray: false,
        systemNotifications: false,
        panelEffects: { alertPulse: false, workingFlow: false },
      }),
    );

    expect(settings.theme).toBe("light");
    expect(settings.closeToTray).toBe(false);
    expect(settings.systemNotifications).toBe(false);
    expect(settings.panelEffects).toEqual({ alertPulse: false, workingFlow: false });
  });

  it("should fall back per panel effect when values have wrong types", () => {
    const settings = parseSettings(
      JSON.stringify({ panelEffects: { alertPulse: "no", workingFlow: false } }),
    );

    expect(settings.panelEffects).toEqual({ alertPulse: true, workingFlow: false });
  });

  it("should accept auto theme and notification preferences", () => {
    const settings = parseSettings(
      JSON.stringify({
        theme: "auto",
        doNotDisturb: true,
        systemNotifications: false,
      }),
    );

    expect(settings.theme).toBe("auto");
    expect(settings.doNotDisturb).toBe(true);
    expect(settings.systemNotifications).toBe(false);
  });

  it("should fall back per field when values have wrong types", () => {
    const defaults = createDefaultSettings();

    const settings = parseSettings(
      JSON.stringify({
        theme: "sepia",
        doNotDisturb: "yes",
        systemNotifications: 1,
        transparency: null,
      }),
    );

    expect(settings.theme).toBe(defaults.theme);
    expect(settings.doNotDisturb).toBe(defaults.doNotDisturb);
    expect(settings.systemNotifications).toBe(defaults.systemNotifications);
    expect(settings.transparency).toBe(defaults.transparency);
  });

  it("should return defaults when JSON is corrupt", () => {
    const settings = parseSettings("{not json");

    expect(settings).toEqual(createDefaultSettings());
  });
});

describe("normalizeShortcut", () => {
  it("should trim and truncate the accelerator", () => {
    const shortcut = `  ${"A".repeat(SHORTCUT_MAX_LENGTH + 10)}  `;

    const normalized = normalizeShortcut(shortcut);

    expect(normalized).toBe("A".repeat(SHORTCUT_MAX_LENGTH));
  });
});
