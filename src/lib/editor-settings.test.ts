import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import {
  editorFontSize, editorTabSize, editorWordWrap, editorMinimap, editorLigatures,
  editorLineHeight, editorLetterSpacing, editorStickyScroll, editorInlayHints,
  editorFormatOnSave, editorBlameAlways, editorGhostText, editorGhostSource, editorVimMode,
  setEditorFontSize, bumpEditorFontSize,
  setEditorTabSize,
  toggleWordWrap, toggleMinimap, toggleLigatures,
  bumpEditorLineHeight, bumpEditorLetterSpacing,
  toggleStickyScroll, toggleInlayHints, toggleFormatOnSave, toggleBlameAlways,
  toggleGhostText, toggleVimMode, setGhostSource,
} from "./editor-settings";

beforeEach(() => localStorage.clear());

describe("setEditorFontSize — clamp + persist", () => {
  it("clamps below min to 8", () => {
    setEditorFontSize(2);
    expect(get(editorFontSize)).toBe(8);
    expect(localStorage.getItem("anvil-editor-fs")).toBe("8");
  });

  it("clamps above max to 32", () => {
    setEditorFontSize(100);
    expect(get(editorFontSize)).toBe(32);
    expect(localStorage.getItem("anvil-editor-fs")).toBe("32");
  });

  it("rounds fractional values", () => {
    setEditorFontSize(14.7);
    expect(get(editorFontSize)).toBe(15);
  });

  it("persists a mid-range value", () => {
    setEditorFontSize(16);
    expect(get(editorFontSize)).toBe(16);
    expect(localStorage.getItem("anvil-editor-fs")).toBe("16");
  });
});

describe("bumpEditorFontSize", () => {
  it("increments the current value", () => {
    setEditorFontSize(13);
    bumpEditorFontSize(1);
    expect(get(editorFontSize)).toBe(14);
  });

  it("does not exceed max", () => {
    setEditorFontSize(32);
    bumpEditorFontSize(5);
    expect(get(editorFontSize)).toBe(32);
  });
});

describe("setEditorTabSize — clamp + persist", () => {
  it("clamps below min to 1", () => {
    setEditorTabSize(0);
    expect(get(editorTabSize)).toBe(1);
    expect(localStorage.getItem("anvil-editor-tab")).toBe("1");
  });

  it("clamps above max to 8", () => {
    setEditorTabSize(20);
    expect(get(editorTabSize)).toBe(8);
    expect(localStorage.getItem("anvil-editor-tab")).toBe("8");
  });

  it("sets a valid value", () => {
    setEditorTabSize(4);
    expect(get(editorTabSize)).toBe(4);
  });
});

describe("toggleWordWrap — flip + persist", () => {
  it("flips false → true and persists '1'", () => {
    editorWordWrap.set(false);
    toggleWordWrap();
    expect(get(editorWordWrap)).toBe(true);
    expect(localStorage.getItem("anvil-editor-wrap")).toBe("1");
  });

  it("flips true → false and persists '0'", () => {
    editorWordWrap.set(true);
    toggleWordWrap();
    expect(get(editorWordWrap)).toBe(false);
    expect(localStorage.getItem("anvil-editor-wrap")).toBe("0");
  });
});

describe("toggleMinimap — flip + persist", () => {
  it("flips and persists", () => {
    editorMinimap.set(true);
    toggleMinimap();
    expect(get(editorMinimap)).toBe(false);
    expect(localStorage.getItem("anvil-editor-minimap")).toBe("0");
  });
});

describe("toggleLigatures — flip + persist", () => {
  it("flips and persists", () => {
    editorLigatures.set(true);
    toggleLigatures();
    expect(get(editorLigatures)).toBe(false);
    expect(localStorage.getItem("anvil-editor-ligatures")).toBe("0");
  });
});

describe("bumpEditorLineHeight — clamp + persist", () => {
  it("does not go below 1.0", () => {
    editorLineHeight.set(1.0);
    bumpEditorLineHeight(-1);
    expect(get(editorLineHeight)).toBe(1.0);
  });

  it("does not exceed 2.4", () => {
    editorLineHeight.set(2.4);
    bumpEditorLineHeight(1);
    expect(get(editorLineHeight)).toBe(2.4);
  });

  it("persists the new value", () => {
    editorLineHeight.set(1.5);
    bumpEditorLineHeight(0.1);
    const stored = localStorage.getItem("anvil-editor-lh");
    expect(stored).not.toBeNull();
    expect(Number(stored)).toBeCloseTo(1.6, 1);
  });
});

describe("bumpEditorLetterSpacing — clamp + persist", () => {
  it("does not go below -2", () => {
    editorLetterSpacing.set(-2);
    bumpEditorLetterSpacing(-5);
    expect(get(editorLetterSpacing)).toBe(-2);
  });

  it("does not exceed 4", () => {
    editorLetterSpacing.set(4);
    bumpEditorLetterSpacing(5);
    expect(get(editorLetterSpacing)).toBe(4);
  });

  it("persists", () => {
    editorLetterSpacing.set(0);
    bumpEditorLetterSpacing(2);
    expect(localStorage.getItem("anvil-editor-ls")).toBe("2");
  });
});

describe("boolean toggles — flip + persist", () => {
  it("toggleStickyScroll flips + persists", () => {
    editorStickyScroll.set(true);
    toggleStickyScroll();
    expect(get(editorStickyScroll)).toBe(false);
    expect(localStorage.getItem("anvil-editor-sticky")).toBe("0");
  });

  it("toggleInlayHints flips + persists", () => {
    editorInlayHints.set(false);
    toggleInlayHints();
    expect(get(editorInlayHints)).toBe(true);
    expect(localStorage.getItem("anvil-editor-inlay")).toBe("1");
  });

  it("toggleFormatOnSave flips + persists", () => {
    editorFormatOnSave.set(true);
    toggleFormatOnSave();
    expect(get(editorFormatOnSave)).toBe(false);
    expect(localStorage.getItem("anvil-editor-fos")).toBe("0");
  });

  it("toggleBlameAlways flips + persists", () => {
    editorBlameAlways.set(false);
    toggleBlameAlways();
    expect(get(editorBlameAlways)).toBe(true);
    expect(localStorage.getItem("anvil-editor-blame")).toBe("1");
  });

  it("toggleGhostText flips + persists", () => {
    editorGhostText.set(false);
    toggleGhostText();
    expect(get(editorGhostText)).toBe(true);
    expect(localStorage.getItem("anvil-editor-ghost")).toBe("1");
  });

  it("toggleVimMode flips + persists", () => {
    editorVimMode.set(false);
    toggleVimMode();
    expect(get(editorVimMode)).toBe(true);
    expect(localStorage.getItem("anvil-editor-vim")).toBe("1");
  });
});

describe("setGhostSource — set + persist", () => {
  it("sets to 'llm' and persists", () => {
    setGhostSource("llm");
    expect(get(editorGhostSource)).toBe("llm");
    expect(localStorage.getItem("anvil-editor-ghost-src")).toBe("llm");
  });

  it("sets back to 'lsp' and persists", () => {
    setGhostSource("lsp");
    expect(get(editorGhostSource)).toBe("lsp");
    expect(localStorage.getItem("anvil-editor-ghost-src")).toBe("lsp");
  });
});
