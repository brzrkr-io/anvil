// Agent tool-use protocol (#53). The agent drives an approval-gated loop by
// emitting fenced tool-call blocks; we parse them, the user approves each, we
// execute and feed the result back, then continue. The protocol is ours, so the
// parser is fully testable. Two tools:
//   ```anvil:run\n<shell command>\n```      → run in cwd, capture output
//   ```anvil:read\n<path>\n```              → read a file into context
// (File edits keep using the existing whole-file fenced block + DiffReview.)

export type ToolKind = "run" | "read";

export interface ToolCall {
  kind: ToolKind;
  arg: string; // command text (run) or path (read)
}

export function parseToolCalls(text: string): ToolCall[] {
  const out: ToolCall[] = [];
  const re = /```anvil:(run|read)\n?([\s\S]*?)```/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) {
    const kind = m[1] as ToolKind;
    const arg = m[2].trim();
    if (arg) out.push({ kind, arg });
  }
  return out;
}

// The system-prompt fragment that teaches the model the protocol. Appended to
// the agent's base system message when tool-use is enabled.
export const TOOL_SYSTEM_PROMPT =
  "You can use tools by emitting fenced blocks. To run a shell command: a " +
  "```anvil:run block containing exactly the command. To read a file: a " +
  "```anvil:read block containing exactly the path. Emit ONE tool call at a " +
  "time and wait for its result (provided back to you) before the next. To " +
  "propose file edits, emit one ```anvil:edit <path> fence per file with the " +
  "COMPLETE new file body; the user reviews each as a diff. When the task is " +
  "done, stop emitting tool calls and give a short summary.";

// Multi-file edit blocks (#31): the agent proposes a full new file body per
// path via ```anvil:edit <path> fences; each becomes a reviewable diff.
export interface EditBlock { path: string; content: string; }
export function parseEditBlocks(text: string): EditBlock[] {
  const out: EditBlock[] = [];
  const re = /```anvil:edit[ \t]+(\S+)\n([\s\S]*?)```/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) out.push({ path: m[1], content: m[2].replace(/\n$/, "") });
  return out;
}

// Format a tool result as the user-role message fed back into the conversation.
export function toolResultMessage(call: ToolCall, output: string): string {
  const label = call.kind === "run" ? `run ${call.arg}` : `read ${call.arg}`;
  return `Tool result (${label}):\n\`\`\`\n${output}\n\`\`\``;
}
