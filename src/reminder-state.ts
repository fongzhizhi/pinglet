/**
 * @ai-generated deepseek-v4-flash
 * 提醒判定：根据上一轮与当前轮的会话状态推导本轮提醒事件，不依赖 DOM。
 */

import { STATE_META, classify, getRowKey, type BoardState, type SessionRow } from "./session-state";

export type AbnormalState = Extract<BoardState, "blocked" | "interrupted">;

export type ReminderEvent = { kind: "alert"; state: AbnormalState } | { kind: "completion" };

export type ReminderTransition = {
  event: ReminderEvent | null;
  nextStates: Map<string, BoardState>;
};

/** 异常态：待处理 / 已中断，需要呼吸灯与提醒。 */
export function isAbnormalState(state: BoardState): state is AbnormalState {
  return state === "blocked" || state === "interrupted";
}

/** priority 数值越小越优先：待处理压过已中断。 */
function pickHigherPriority(
  current: AbnormalState | null,
  candidate: AbnormalState,
): AbnormalState {
  if (current === null) {
    return candidate;
  }
  return STATE_META[candidate].priority < STATE_META[current].priority ? candidate : current;
}

/**
 * 计算本轮提醒事件：启动首轮只播种状态，不补闪既有会话；
 * 后续新出现的异常会话、状态突变为异常、进行中 → 已完成都会产生提醒。
 * NOTE: 同一次轮询里异常优先，完成提示让位。
 */
export function detectReminderEvent(
  rows: SessionRow[],
  previousStates: ReadonlyMap<string, BoardState>,
  hasRenderedSessions: boolean,
): ReminderTransition {
  const nextStates = new Map<string, BoardState>();
  let alertState: AbnormalState | null = null;
  let hasCompletion = false;

  for (const row of rows) {
    const key = getRowKey(row);
    const state = classify(row);
    nextStates.set(key, state);

    const previous = previousStates.get(key);
    if (previous === undefined) {
      // 启动时已存在的会话不补闪；后续新出现的异常会话仍要提醒
      if (hasRenderedSessions && isAbnormalState(state)) {
        alertState = pickHigherPriority(alertState, state);
      }
      continue;
    }

    if (previous === state) {
      continue;
    }

    if (isAbnormalState(state)) {
      alertState = pickHigherPriority(alertState, state);
    } else if (previous === "working" && state === "done") {
      hasCompletion = true;
    }
  }

  const event: ReminderEvent | null =
    alertState !== null
      ? { kind: "alert", state: alertState }
      : hasCompletion
        ? { kind: "completion" }
        : null;

  return { event, nextStates };
}
