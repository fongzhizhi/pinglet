import { describe, expect, it } from "vitest";
import {
  FOLDER_TAG_COLOR_COUNT,
  assignFolderColorSlots,
  folderColorKey,
  folderTagColor,
} from "./folder-colors";

describe("folderColorKey", () => {
  it("should lower-case and trim trailing separators when cwd is a Windows path", () => {
    expect(folderColorKey("D:\\Workspace\\Pinglet\\")).toBe("d:\\workspace\\pinglet");
    expect(folderColorKey("D:/Workspace/Pinglet///")).toBe("d:/workspace/pinglet");
  });

  it("should return the unknown key when cwd is null or blank", () => {
    expect(folderColorKey(null)).toBe("未知目录");
    expect(folderColorKey("   ")).toBe("未知目录");
    expect(folderColorKey("\\\\")).toBe("未知目录");
  });
});

describe("assignFolderColorSlots", () => {
  it("should keep slots stable when called twice with the same keys", () => {
    const keys = ["a", "b", "c"];

    const first = assignFolderColorSlots(keys, new Map());
    const second = assignFolderColorSlots(keys, first);

    expect(second).toEqual(first);
  });

  it("should reuse previous slots when a key is already assigned", () => {
    const previous = new Map([
      ["a", 5],
      ["b", 4],
    ]);

    const slots = assignFolderColorSlots(["a", "b", "c"], previous);

    expect(slots.get("a")).toBe(5);
    expect(slots.get("b")).toBe(4);
  });

  it("should assign unique slots when folders are fewer than the palette size", () => {
    const keys = Array.from({ length: FOLDER_TAG_COLOR_COUNT }, (_, index) => `folder-${index}`);

    const slots = assignFolderColorSlots(keys, new Map());

    expect(new Set(slots.values()).size).toBe(FOLDER_TAG_COLOR_COUNT);
  });

  it("should reuse slots and stay in range when folders exceed the palette size", () => {
    const keys = Array.from(
      { length: FOLDER_TAG_COLOR_COUNT + 2 },
      (_, index) => `folder-${index}`,
    );

    const slots = assignFolderColorSlots(keys, new Map());

    expect(slots.size).toBe(keys.length);
    expect(new Set(slots.values()).size).toBe(FOLDER_TAG_COLOR_COUNT);
    for (const slot of slots.values()) {
      expect(slot).toBeGreaterThanOrEqual(0);
      expect(slot).toBeLessThan(FOLDER_TAG_COLOR_COUNT);
    }
  });

  it("should drop vanished keys when the next round no longer sees them", () => {
    const previous = assignFolderColorSlots(["a", "b"], new Map());

    const slots = assignFolderColorSlots(["a"], previous);

    expect([...slots.keys()]).toEqual(["a"]);
  });
});

describe("folderTagColor", () => {
  it("should return the css variable when the key has a slot", () => {
    const slots = new Map([["a", 3]]);

    expect(folderTagColor("a", slots)).toBe("var(--color-tag-3)");
  });

  it("should return the fallback color when the key has no slot", () => {
    expect(folderTagColor("missing", new Map())).toBe("var(--color-text-sub)");
  });
});
