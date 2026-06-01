// #13 Command history — captured by accumulating typed input per terminal and
// committing on Enter. Powers click-to-rerun + a searchable history palette.
// Persisted (capped) so history survives restarts.
const KEY = "anvil-cmd-history";
const CAP = 500;

function load(): string[] {
  if (typeof localStorage === "undefined") return [];
  try { return JSON.parse(localStorage.getItem(KEY) || "[]"); } catch { return []; }
}
let history: string[] = load();

// Per-terminal in-progress line buffers (raw keystrokes since the last Enter).
const buffers = new Map<string, string>();

export function feedInput(id: string, data: string) {
  let buf = buffers.get(id) ?? "";
  for (const ch of data) {
    if (ch === "\r" || ch === "\n") {
      commit(buf);
      buf = "";
    } else if (ch === "\x7f" || ch === "\b") {
      buf = buf.slice(0, -1);
    } else if (ch === "\x03" || ch === "\x15") {
      buf = ""; // Ctrl-C / Ctrl-U: discard the line
    } else if (ch >= " ") {
      buf += ch;
    }
  }
  buffers.set(id, buf);
}

function commit(line: string) {
  const cmd = line.trim();
  if (!cmd || cmd.length > 400) return;
  if (history[history.length - 1] === cmd) return; // dedupe consecutive
  history.push(cmd);
  if (history.length > CAP) history = history.slice(-CAP);
  try { localStorage.setItem(KEY, JSON.stringify(history)); } catch { /* ignore */ }
}

export function getHistory(): string[] { return history; }
export function clearHistory() { history = []; try { localStorage.removeItem(KEY); } catch { /* ignore */ } }
