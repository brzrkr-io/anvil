import { writable } from "svelte/store";

// Set this to pre-fill the agent input box and focus it (#55 "explain this
// error" and similar one-click-to-agent actions). The AgentPanel consumes the
// value and clears it.
export const agentSeed = writable<string>("");
