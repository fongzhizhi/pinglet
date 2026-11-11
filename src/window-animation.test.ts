import { describe, expect, it } from "vitest";
import {
  beginPanelHide,
  cancelPanelHide,
  completePanelHide,
  createPanelAnimationState,
  markPanelHidden,
  markPanelShown,
} from "./window-animation";

describe("panel animation state", () => {
  it("should not animate the first show when the app just started", () => {
    const state = createPanelAnimationState();

    expect(markPanelShown(state)).toBe(false);
    expect(state.collapsed).toBe(false);
  });

  it("should animate when showing after the window was hidden", () => {
    const state = createPanelAnimationState();
    markPanelHidden(state);

    expect(markPanelShown(state)).toBe(true);
    expect(state.collapsed).toBe(false);
    expect(state.hasBeenHidden).toBe(true);
  });

  it("should cancel a pending hide when the window is shown again", () => {
    const state = createPanelAnimationState();
    markPanelHidden(state);
    beginPanelHide(state, 7);

    expect(markPanelShown(state)).toBe(true);
    expect(state.pendingHideGeneration).toBeNull();
    expect(completePanelHide(state, 7)).toBe(false);
  });

  it("should reject an outdated hide generation", () => {
    const state = createPanelAnimationState();
    markPanelHidden(state);
    beginPanelHide(state, 1);
    beginPanelHide(state, 2);

    expect(completePanelHide(state, 1)).toBe(false);
    expect(state.pendingHideGeneration).toBe(2);
    expect(completePanelHide(state, 2)).toBe(true);
  });

  it("should drop the pending hide once the window reports hidden", () => {
    const state = createPanelAnimationState();
    beginPanelHide(state, 3);
    markPanelHidden(state);

    expect(completePanelHide(state, 3)).toBe(false);
    expect(state.pendingHideGeneration).toBeNull();
  });

  it("should clear the pending hide when cancelled", () => {
    const state = createPanelAnimationState();
    beginPanelHide(state, 4);
    cancelPanelHide(state);

    expect(completePanelHide(state, 4)).toBe(false);
  });
});
