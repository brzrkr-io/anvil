import { writable } from "svelte/store";

// Set this to pre-fill the agent input box and focus it (#55 "explain this
// error" and similar one-click-to-agent actions). The AgentPanel consumes the
// value and clears it.
export const agentSeed = writable<string>("");

// Like agentSeed, but signals an agent-driven investigation: the panel enables
// Agent (tool-use) mode and auto-sends, so one click from a failing resource
// starts a live, approval-gated investigation. Consumed + cleared by AgentPanel.
export const agentInvestigate = writable<string>("");
