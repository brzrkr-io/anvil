import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import { agentQueue, enqueueAgent, dequeueAgent, removeQueued, clearQueue } from "./agent-queue";

beforeEach(() => agentQueue.set([]));

describe("enqueueAgent", () => {
  it("adds a task with the trimmed prompt", () => {
    enqueueAgent("  deploy prod  ");
    const q = get(agentQueue);
    expect(q).toHaveLength(1);
    expect(q[0].prompt).toBe("deploy prod");
  });

  it("ignores blank prompts", () => {
    enqueueAgent("   ");
    expect(get(agentQueue)).toHaveLength(0);
  });

  it("assigns unique ids to each task", () => {
    enqueueAgent("a");
    enqueueAgent("b");
    const ids = get(agentQueue).map((t) => t.id);
    expect(new Set(ids).size).toBe(2);
  });

  it("appends in FIFO order", () => {
    enqueueAgent("first");
    enqueueAgent("second");
    const q = get(agentQueue);
    expect(q[0].prompt).toBe("first");
    expect(q[1].prompt).toBe("second");
  });
});

describe("dequeueAgent", () => {
  it("returns and removes the head of the queue", () => {
    enqueueAgent("first");
    enqueueAgent("second");
    const task = dequeueAgent();
    expect(task?.prompt).toBe("first");
    expect(get(agentQueue)).toHaveLength(1);
    expect(get(agentQueue)[0].prompt).toBe("second");
  });

  it("returns null when the queue is empty", () => {
    expect(dequeueAgent()).toBeNull();
  });
});

describe("removeQueued", () => {
  it("removes only the task with the given id", () => {
    enqueueAgent("keep");
    enqueueAgent("remove");
    const removeId = get(agentQueue)[1].id;
    removeQueued(removeId);
    const q = get(agentQueue);
    expect(q).toHaveLength(1);
    expect(q[0].prompt).toBe("keep");
  });

  it("is a no-op for an unknown id", () => {
    enqueueAgent("x");
    removeQueued(999999);
    expect(get(agentQueue)).toHaveLength(1);
  });
});

describe("clearQueue", () => {
  it("empties the queue", () => {
    enqueueAgent("a");
    enqueueAgent("b");
    clearQueue();
    expect(get(agentQueue)).toHaveLength(0);
  });
});
