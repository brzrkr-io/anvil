import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import {
  termFontSize, termCursorBlink, termCursorStyle, termLineHeight, termLetterSpacing, termScrollback,
  CURSOR_STYLES,
  setTermFontSize, bumpTermFontSize,
  toggleTermBlink,
  setTermCursorStyle,
  bumpTermLineHeight,
  bumpTermLetterSpacing,
  setTermScrollback,
} from "./terminal-settings";

beforeEach(() => localStorage.clear());

describe("setTermFontSize — clamp + persist", () => {
  it("clamps below 8 to 8", () => {
    setTermFontSize(2);
    expect(get(termFontSize)).toBe(8);
    expect(localStorage.getItem("anvil-term-fs")).toBe("8");
  });

  it("clamps above 28 to 28", () => {
    setTermFontSize(100);
    expect(get(termFontSize)).toBe(28);
    expect(localStorage.getItem("anvil-term-fs")).toBe("28");
  });

  it("rounds fractional values", () => {
    setTermFontSize(13.6);
    expect(get(termFontSize)).toBe(14);
  });

  it("persists a mid-range value", () => {
    setTermFontSize(14);
    expect(get(termFontSize)).toBe(14);
    expect(localStorage.getItem("anvil-term-fs")).toBe("14");
  });
});

describe("bumpTermFontSize", () => {
  it("increments by delta", () => {
    setTermFontSize(13);
    bumpTermFontSize(2);
    expect(get(termFontSize)).toBe(15);
  });

  it("does not exceed 28", () => {
    setTermFontSize(28);
    bumpTermFontSize(5);
    expect(get(termFontSize)).toBe(28);
  });
});

describe("toggleTermBlink — flip + persist", () => {
  it("flips true → false and persists '0'", () => {
    termCursorBlink.set(true);
    toggleTermBlink();
    expect(get(termCursorBlink)).toBe(false);
    expect(localStorage.getItem("anvil-term-blink")).toBe("0");
  });

  it("flips false → true and persists '1'", () => {
    termCursorBlink.set(false);
    toggleTermBlink();
    expect(get(termCursorBlink)).toBe(true);
    expect(localStorage.getItem("anvil-term-blink")).toBe("1");
  });
});

describe("setTermCursorStyle — set + persist", () => {
  it("sets each cursor style and persists", () => {
    for (const s of CURSOR_STYLES) {
      setTermCursorStyle(s);
      expect(get(termCursorStyle)).toBe(s);
      expect(localStorage.getItem("anvil-term-cursor")).toBe(s);
    }
  });
});

describe("bumpTermLineHeight — clamp + persist", () => {
  it("does not go below 1.0", () => {
    termLineHeight.set(1.0);
    bumpTermLineHeight(-1);
    expect(get(termLineHeight)).toBe(1.0);
  });

  it("does not exceed 2.0", () => {
    termLineHeight.set(2.0);
    bumpTermLineHeight(1);
    expect(get(termLineHeight)).toBe(2.0);
  });

  it("persists mid-range value", () => {
    termLineHeight.set(1.4);
    bumpTermLineHeight(0.1);
    const stored = Number(localStorage.getItem("anvil-term-lh"));
    expect(stored).toBeCloseTo(1.5, 1);
  });
});

describe("bumpTermLetterSpacing — clamp + persist", () => {
  it("does not go below -2", () => {
    termLetterSpacing.set(-2);
    bumpTermLetterSpacing(-5);
    expect(get(termLetterSpacing)).toBe(-2);
  });

  it("does not exceed 4", () => {
    termLetterSpacing.set(4);
    bumpTermLetterSpacing(5);
    expect(get(termLetterSpacing)).toBe(4);
  });

  it("persists", () => {
    termLetterSpacing.set(1);
    bumpTermLetterSpacing(1);
    expect(localStorage.getItem("anvil-term-ls")).toBe("2");
  });
});

describe("setTermScrollback — clamp + persist", () => {
  it("clamps below 0 to 0", () => {
    setTermScrollback(-100);
    expect(get(termScrollback)).toBe(0);
    expect(localStorage.getItem("anvil-term-scrollback")).toBe("0");
  });

  it("clamps above the ceiling to 500000", () => {
    setTermScrollback(999999);
    expect(get(termScrollback)).toBe(500000);
    expect(localStorage.getItem("anvil-term-scrollback")).toBe("500000");
  });

  it("persists a typical value", () => {
    setTermScrollback(5000);
    expect(get(termScrollback)).toBe(5000);
    expect(localStorage.getItem("anvil-term-scrollback")).toBe("5000");
  });
});
