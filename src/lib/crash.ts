// #98 Crash / feedback reporter. Captures uncaught errors and rejections to a
// local ring buffer (never auto-sent), and builds a diagnostics blob the user
// can copy into a bug report.
const KEY = "anvil-crashes";
const CAP = 50;

export type CrashEntry = { ts: number; kind: string; message: string; stack?: string };

export function getCrashes(): CrashEntry[] {
  if (typeof localStorage === "undefined") return [];
  try { return JSON.parse(localStorage.getItem(KEY) || "[]"); } catch { return []; }
}
export function clearCrashes() {
  if (typeof localStorage !== "undefined") localStorage.removeItem(KEY);
}

function record(kind: string, message: string, stack?: string) {
  try {
    const list = getCrashes();
    list.push({ ts: Date.now(), kind, message: message.slice(0, 500), stack: stack?.slice(0, 2000) });
    localStorage.setItem(KEY, JSON.stringify(list.slice(-CAP)));
  } catch { /* ignore */ }
}

// First stack frame, so a surfaced error can name where it came from.
export function originFrame(stack?: string): string {
  if (!stack) return "";
  const lines = stack.split("\n").map((l) => l.trim()).filter(Boolean);
  const frame = lines.find((l) => /\.svelte|\/src\/|\/_app\/|\.ts:|\.js:|@/.test(l) && !/^[A-Za-z]*Error\b/.test(l)) ?? "";
  return frame.replace(/^at\s+/, "").slice(0, 120);
}

let installed = false;
export function installCrashHandlers(onCapture?: (kind: string, message: string, origin: string) => void) {
  if (installed || typeof window === "undefined") return;
  installed = true;
  window.addEventListener("error", (e) => {
    const msg = e.message || "unknown error";
    const stack = (e.error as Error | undefined)?.stack;
    record("error", msg, stack);
    console.error("[anvil] uncaught error:", msg, "\n", stack ?? "(no stack)");
    onCapture?.("error", msg, originFrame(stack));
  });
  window.addEventListener("unhandledrejection", (e) => {
    const r = e.reason;
    const msg = typeof r === "string" ? r : (r?.message ?? String(r));
    record("promise", msg, r?.stack);
    console.error("[anvil] unhandled rejection:", msg, "\n", r?.stack ?? "(no stack)");
    onCapture?.("promise", msg, originFrame(r?.stack));
  });
}

// A copy-pasteable diagnostics report for a bug filing. No secrets — just env
// and recent crash signatures.
export function diagnosticsReport(version: string): string {
  const crashes = getCrashes();
  const lines = [
    `Anvil v${version}`,
    `UA: ${typeof navigator !== "undefined" ? navigator.userAgent : "n/a"}`,
    `Recent crashes: ${crashes.length}`,
    ...crashes.slice(-10).map((c) => `  [${new Date(c.ts).toISOString()}] ${c.kind}: ${c.message}`),
  ];
  return lines.join("\n");
}
