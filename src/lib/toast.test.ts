import { describe, it, expect, beforeEach, vi, afterEach } from "vitest";
import { get } from "svelte/store";
import { toasts, toast, dismiss, notifications, markAllRead, clearNotifications } from "./toast";

beforeEach(() => {
  vi.useFakeTimers();
  toasts.set([]);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("toast — push behavior", () => {
  it("adds a toast with the given text and kind", () => {
    toast("hello", "info");
    const all = get(toasts);
    expect(all).toHaveLength(1);
    expect(all[0].text).toBe("hello");
    expect(all[0].kind).toBe("info");
  });

  it("defaults kind to 'info'", () => {
    toast("default kind");
    expect(get(toasts)[0].kind).toBe("info");
  });

  it("assigns unique ids to multiple toasts", () => {
    toast("a");
    toast("b");
    const ids = get(toasts).map((t) => t.id);
    expect(new Set(ids).size).toBe(2);
  });

  it("stacks multiple toasts", () => {
    toast("one", "success");
    toast("two", "error");
    expect(get(toasts)).toHaveLength(2);
  });
});

describe("dismiss", () => {
  it("removes the toast with the given id", () => {
    toast("to remove", "info");
    const id = get(toasts)[0].id;
    dismiss(id);
    expect(get(toasts)).toHaveLength(0);
  });

  it("leaves other toasts intact", () => {
    toast("keep", "success");
    toast("remove", "error");
    const removeId = get(toasts)[1].id;
    dismiss(removeId);
    const remaining = get(toasts);
    expect(remaining).toHaveLength(1);
    expect(remaining[0].text).toBe("keep");
  });
});

describe("auto-expire via TTL", () => {
  it("auto-dismisses after the default TTL (3500ms)", () => {
    toast("expires", "info");
    expect(get(toasts)).toHaveLength(1);
    vi.advanceTimersByTime(3500);
    expect(get(toasts)).toHaveLength(0);
  });

  it("auto-dismisses after a custom TTL", () => {
    toast("short", "info", 1000);
    vi.advanceTimersByTime(999);
    expect(get(toasts)).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(get(toasts)).toHaveLength(0);
  });

  it("does not dismiss before TTL elapses", () => {
    toast("pending", "info", 5000);
    vi.advanceTimersByTime(4999);
    expect(get(toasts)).toHaveLength(1);
  });
});

describe("notification history (#94)", () => {
  beforeEach(() => {
    clearNotifications();
    localStorage.clear();
  });

  it("archives every toast newest-first with an unread flag", () => {
    toast("first", "info", 3500, 1000);
    toast("second", "error", 3500, 2000);
    const all = get(notifications);
    expect(all.map((n) => n.text)).toEqual(["second", "first"]);
    expect(all[0]).toMatchObject({ kind: "error", ts: 2000, read: false });
  });

  it("persists to localStorage so history survives reload", () => {
    toast("kept", "success", 3500, 5000);
    const raw = JSON.parse(localStorage.getItem("anvil-notifications") || "[]");
    expect(raw[0]).toMatchObject({ text: "kept", kind: "success" });
  });

  it("markAllRead flips every unread entry", () => {
    toast("a", "info", 3500, 1);
    toast("b", "info", 3500, 2);
    markAllRead();
    expect(get(notifications).every((n) => n.read)).toBe(true);
  });

  it("clearNotifications empties the feed and storage", () => {
    toast("gone", "info", 3500, 1);
    clearNotifications();
    expect(get(notifications)).toEqual([]);
    expect(localStorage.getItem("anvil-notifications")).toBe("[]");
  });
});
