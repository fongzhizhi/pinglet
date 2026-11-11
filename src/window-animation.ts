/**
 * @ai-generated deepseek-v4-flash
 * 主窗口显隐动画的纯状态机：判断是否播放展开动画，并让过期的延迟隐藏失效。
 */

export type PanelAnimationState = {
  /** 面板当前是否处于收起态（窗口隐藏期间保留，复显时从收起态展开）。 */
  collapsed: boolean;
  /** 正在等待完成的隐藏代号；Rust 侧 generation 的镜像。 */
  pendingHideGeneration: number | null;
  /** 是否收到过窗口隐藏事件；首次启动不播放展开动画。 */
  hasBeenHidden: boolean;
};

export function createPanelAnimationState(): PanelAnimationState {
  return {
    collapsed: false,
    pendingHideGeneration: null,
    hasBeenHidden: false,
  };
}

/** 窗口已经隐藏：保留收起态，并让所有待执行的隐藏代号失效。 */
export function markPanelHidden(state: PanelAnimationState): void {
  state.collapsed = true;
  state.pendingHideGeneration = null;
  state.hasBeenHidden = true;
}

/** 收到显示请求或窗口恢复可见；返回是否需要播放展开动画。 */
export function markPanelShown(state: PanelAnimationState): boolean {
  const shouldAnimate = state.collapsed && state.hasBeenHidden;
  state.collapsed = false;
  state.pendingHideGeneration = null;
  return shouldAnimate;
}

/** 开始收起动画，记录本次隐藏代号。 */
export function beginPanelHide(state: PanelAnimationState, generation: number): void {
  state.collapsed = true;
  state.pendingHideGeneration = generation;
}

/** 收起动画结束；代号已过期（期间重新显示或又发起新的隐藏）时返回 false。 */
export function completePanelHide(state: PanelAnimationState, generation: number): boolean {
  if (state.pendingHideGeneration !== generation) {
    return false;
  }
  state.pendingHideGeneration = null;
  return true;
}

/** 取消待执行的隐藏（不做任何窗口操作）。 */
export function cancelPanelHide(state: PanelAnimationState): void {
  state.pendingHideGeneration = null;
}
