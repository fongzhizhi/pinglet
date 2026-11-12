/**
 * @ai-generated deepseek-v4-flash
 * 目录标签配色：同一目录稳定映射到固定色板槽位，不依赖 DOM。
 */

// 目录标签色板槽位数，与 styles.css 的 --color-tag-0..7 一一对应
export const FOLDER_TAG_COLOR_COUNT = 8;
// FNV-1a 哈希常量，用于让同一目录稳定落到同一个色板起点
const FNV_OFFSET_BASIS = 0x811c9dc5;
const FNV_PRIME = 0x01000193;
const UNKNOWN_FOLDER_KEY = "未知目录";
const UNKNOWN_TAG_COLOR = "var(--color-text-sub)";
const TRAILING_SEPARATOR_PATTERN = /[\\/]+$/;

/** 目录标签配色键：同一路径稳定同色，未知目录统一兜底色。 */
export function folderColorKey(cwd: string | null): string {
  const path = cwd?.trim().replace(TRAILING_SEPARATOR_PATTERN, "");
  return path ? path.toLowerCase() : UNKNOWN_FOLDER_KEY;
}

/** FNV-1a 32 位哈希，避免随机数导致每次刷新目录变色。 */
function hashString(value: string): number {
  let hash = FNV_OFFSET_BASIS;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, FNV_PRIME);
  }
  return hash >>> 0;
}

/**
 * 为当前可见目录分配色板槽位：沿用 previous 中的分配，新目录从哈希位置线性探测空槽。
 * 目录数超过色板容量时允许复用，避免无限扩张。
 */
export function assignFolderColorSlots(
  keys: string[],
  previous: ReadonlyMap<string, number>,
): Map<string, number> {
  const uniqueKeys = [...new Set(keys)];
  const next = new Map<string, number>();
  const usedSlots = new Set<number>();

  for (const key of uniqueKeys) {
    const slot = previous.get(key);
    if (slot !== undefined) {
      next.set(key, slot);
      usedSlots.add(slot);
    }
  }

  for (const key of uniqueKeys) {
    if (next.has(key)) {
      continue;
    }
    let slot = hashString(key) % FOLDER_TAG_COLOR_COUNT;
    const startSlot = slot;
    while (usedSlots.has(slot)) {
      slot = (slot + 1) % FOLDER_TAG_COLOR_COUNT;
      if (slot === startSlot) {
        break;
      }
    }
    usedSlots.add(slot);
    next.set(key, slot);
  }

  return next;
}

export function folderTagColor(key: string, slots: ReadonlyMap<string, number>): string {
  const slot = slots.get(key);
  return slot === undefined ? UNKNOWN_TAG_COLOR : `var(--color-tag-${slot})`;
}
