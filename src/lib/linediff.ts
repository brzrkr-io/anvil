// Line-level diff for agent inline edits (#54). Computes the changed hunks
// between the current file and the agent's proposed replacement so each hunk can
// be accepted or rejected independently. Pure + testable; an LCS backs it.

export interface DiffHunk {
  oldStart: number; // 0-based index into old lines where the hunk's change begins
  oldLines: string[]; // the removed/replaced old lines (may be empty for a pure insert)
  newLines: string[]; // the added new lines (may be empty for a pure deletion)
}

// Longest-common-subsequence over lines → a flat op list, then group runs of
// non-equal ops (with their adjacent removes+adds) into hunks.
export function diffLines(oldText: string, newText: string): DiffHunk[] {
  const a = oldText.split("\n");
  const b = newText.split("\n");
  const n = a.length, m = b.length;
  // DP table of LCS lengths.
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  // Backtrack into ops: "eq" | "del" | "add".
  type Op = { kind: "eq" | "del" | "add"; line: string; oi: number };
  const ops: Op[] = [];
  let i = 0, j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) { ops.push({ kind: "eq", line: a[i], oi: i }); i++; j++; }
    else if (dp[i + 1][j] >= dp[i][j + 1]) { ops.push({ kind: "del", line: a[i], oi: i }); i++; }
    else { ops.push({ kind: "add", line: b[j], oi: i }); j++; }
  }
  while (i < n) { ops.push({ kind: "del", line: a[i], oi: i }); i++; }
  while (j < m) { ops.push({ kind: "add", line: b[j], oi: i }); j++; }

  // Group consecutive del/add ops into hunks.
  const hunks: DiffHunk[] = [];
  let k = 0;
  while (k < ops.length) {
    if (ops[k].kind === "eq") { k++; continue; }
    const start = ops[k].oi;
    const oldLines: string[] = [];
    const newLines: string[] = [];
    while (k < ops.length && ops[k].kind !== "eq") {
      if (ops[k].kind === "del") oldLines.push(ops[k].line);
      else newLines.push(ops[k].line);
      k++;
    }
    hunks.push({ oldStart: start, oldLines, newLines });
  }
  return hunks;
}

// Rebuild the file applying only the accepted hunks (rejected hunks keep the
// old lines). `accepted[i]` corresponds to `hunks[i]`.
export function applyHunks(oldText: string, hunks: DiffHunk[], accepted: boolean[]): string {
  const a = oldText.split("\n");
  const out: string[] = [];
  let cursor = 0;
  hunks.forEach((h, idx) => {
    while (cursor < h.oldStart) out.push(a[cursor++]);
    if (accepted[idx]) {
      out.push(...h.newLines);
    } else {
      out.push(...h.oldLines);
    }
    cursor += h.oldLines.length;
  });
  while (cursor < a.length) out.push(a[cursor++]);
  return out.join("\n");
}
