import { describe, it, expect, beforeEach } from "vitest";
import {
  comboOf,
  comboFor,
  setKeyOverride,
  clearKeyOverride,
  KEY_ACTIONS,
  applyKeymapPreset,
  KEY_PRESETS,
  keyOverrides,
} from "./keymap";

// Helper: construct a minimal KeyboardEvent-like object
function fakeEvent(fields: {
  key: string;
  metaKey?: boolean;
  ctrlKey?: boolean;
  altKey?: boolean;
  shiftKey?: boolean;
}): KeyboardEvent {
  return {
    key: fields.key,
    metaKey: fields.metaKey ?? false,
    ctrlKey: fields.ctrlKey ?? false,
    altKey: fields.altKey ?? false,
    shiftKey: fields.shiftKey ?? false,
  } as unknown as KeyboardEvent;
}

// Reset overrides between tests so state doesn't leak
function resetOverrides() {
  for (const a of KEY_ACTIONS) clearKeyOverride(a.id);
}

describe("comboOf", () => {
  it("produces canonical string for a plain key", () => {
    expect(comboOf(fakeEvent({ key: "t" }))).toBe("T");
  });

  it("includes ⌘ for metaKey", () => {
    expect(comboOf(fakeEvent({ key: "t", metaKey: true }))).toBe("⌘T");
  });

  it("includes all modifiers in canonical order (⌘⌃⌥⇧)", () => {
    const combo = comboOf(fakeEvent({ key: "p", metaKey: true, shiftKey: true }));
    expect(combo).toBe("⌘⇧P");
  });

  it("normalises space to 'Space'", () => {
    expect(comboOf(fakeEvent({ key: " " }))).toBe("Space");
  });

  it("leaves multi-character key names (Enter, Escape) unchanged", () => {
    expect(comboOf(fakeEvent({ key: "Enter" }))).toBe("Enter");
    expect(comboOf(fakeEvent({ key: "Escape" }))).toBe("Escape");
  });

  it("uppercases single-character keys", () => {
    expect(comboOf(fakeEvent({ key: "k" }))).toBe("K");
    expect(comboOf(fakeEvent({ key: "K" }))).toBe("K");
  });

  it("includes ⌃ for ctrlKey", () => {
    expect(comboOf(fakeEvent({ key: "c", ctrlKey: true }))).toBe("⌃C");
  });

  it("includes ⌥ for altKey", () => {
    expect(comboOf(fakeEvent({ key: "f", altKey: true }))).toBe("⌥F");
  });
});

describe("comboFor", () => {
  beforeEach(resetOverrides);

  it("returns the default combo when no override is set", () => {
    expect(comboFor("new-terminal", {})).toBe("⌘T");
  });

  it("returns override when one is set", () => {
    setKeyOverride("new-terminal", "⌘⇧N");
    let overrides: Record<string, string> = {};
    keyOverrides.subscribe((v) => { overrides = v; })();
    expect(comboFor("new-terminal", overrides)).toBe("⌘⇧N");
  });

  it("override is reflected after clearKeyOverride", () => {
    setKeyOverride("new-terminal", "⌘⇧N");
    clearKeyOverride("new-terminal");
    let overrides: Record<string, string> = {};
    keyOverrides.subscribe((v) => { overrides = v; })();
    expect(comboFor("new-terminal", overrides)).toBe("⌘T");
  });

  it("returns empty string for unknown action id", () => {
    expect(comboFor("nonexistent-action", {})).toBe("");
  });

  it("every KEY_ACTIONS entry has a non-empty default combo", () => {
    for (const action of KEY_ACTIONS) {
      expect(comboFor(action.id, {})).toBeTruthy();
    }
  });
});

describe("applyKeymapPreset", () => {
  beforeEach(resetOverrides);

  it("returns true for a known preset name", () => {
    expect(applyKeymapPreset("VS Code")).toBe(true);
  });

  it("returns false for an unknown preset name", () => {
    expect(applyKeymapPreset("DoesNotExist")).toBe(false);
  });

  it("applying 'Anvil (default)' preset yields empty overrides", () => {
    // First set something
    setKeyOverride("command-palette", "⌘⇧X");
    applyKeymapPreset("Anvil (default)");
    let overrides: Record<string, string> = {};
    keyOverrides.subscribe((v) => { overrides = v; })();
    expect(Object.keys(overrides)).toHaveLength(0);
  });

  it("VS Code preset overrides command-palette to ⌘⇧P", () => {
    applyKeymapPreset("VS Code");
    let overrides: Record<string, string> = {};
    keyOverrides.subscribe((v) => { overrides = v; })();
    expect(comboFor("command-palette", overrides)).toBe("⌘⇧P");
  });

  it("every KEY_PRESETS entry is a valid preset name", () => {
    for (const name of Object.keys(KEY_PRESETS)) {
      expect(applyKeymapPreset(name)).toBe(true);
    }
  });
});
