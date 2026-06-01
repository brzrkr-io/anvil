import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import {
  telemetryEnabled,
  setTelemetry,
  toggleTelemetry,
  getEvents,
  clearEvents,
  logEvent,
} from "./telemetry";

beforeEach(() => {
  localStorage.clear();
  // Reset the store to default off state
  setTelemetry(false);
});

describe("setTelemetry", () => {
  it("enables telemetry and persists to localStorage", () => {
    setTelemetry(true);
    expect(get(telemetryEnabled)).toBe(true);
    expect(localStorage.getItem("anvil-telemetry-on")).toBe("1");
  });

  it("disables telemetry and persists to localStorage", () => {
    setTelemetry(true);
    setTelemetry(false);
    expect(get(telemetryEnabled)).toBe(false);
    expect(localStorage.getItem("anvil-telemetry-on")).toBe("0");
  });
});

describe("toggleTelemetry", () => {
  it("flips from false to true", () => {
    setTelemetry(false);
    toggleTelemetry();
    expect(get(telemetryEnabled)).toBe(true);
  });

  it("flips from true to false", () => {
    setTelemetry(true);
    toggleTelemetry();
    expect(get(telemetryEnabled)).toBe(false);
  });
});

describe("logEvent", () => {
  it("does not record events when telemetry is off", () => {
    setTelemetry(false);
    logEvent("page_view");
    expect(getEvents()).toHaveLength(0);
  });

  it("records events when telemetry is on", () => {
    setTelemetry(true);
    logEvent("page_view");
    const events = getEvents();
    expect(events).toHaveLength(1);
    expect(events[0].name).toBe("page_view");
  });

  it("records optional data payload", () => {
    setTelemetry(true);
    logEvent("action", { target: "button" });
    const events = getEvents();
    expect(events[0].data).toEqual({ target: "button" });
  });

  it("includes a numeric timestamp", () => {
    setTelemetry(true);
    const before = Date.now();
    logEvent("ts_check");
    const after = Date.now();
    const ts = getEvents()[0].ts;
    expect(ts).toBeGreaterThanOrEqual(before);
    expect(ts).toBeLessThanOrEqual(after);
  });

  it("caps the ring at 500 events without losing the most recent", () => {
    setTelemetry(true);
    for (let i = 0; i < 505; i++) logEvent(`evt-${i}`);
    const events = getEvents();
    expect(events.length).toBeLessThanOrEqual(500);
    // Most recent event should be the last one logged
    expect(events[events.length - 1].name).toBe("evt-504");
  });
});

describe("getEvents / clearEvents", () => {
  it("returns empty array with no events", () => {
    expect(getEvents()).toEqual([]);
  });

  it("clearEvents removes all stored events", () => {
    setTelemetry(true);
    logEvent("a");
    logEvent("b");
    clearEvents();
    expect(getEvents()).toHaveLength(0);
  });

  it("returns empty array when localStorage has corrupt JSON", () => {
    localStorage.setItem("anvil-telemetry", "not-json{{{");
    expect(getEvents()).toEqual([]);
  });
});
