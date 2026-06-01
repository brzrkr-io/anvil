// Pure git-output parsers for the Source Control view. The Rust backend shells
// `git` and returns raw text; these turn it into typed records. Ported from the
// old Zig git model.

const US = ""; // unit separator used in the git log --pretty format

export interface Commit {
  hash: string;
  short: string;
  author: string;
  email: string;
  ts: number;
  parents: string[];
  refs: string;
  subject: string;
}

export function parseLog(raw: string): Commit[] {
  const out: Commit[] = [];
  for (const line of raw.split("\n")) {
    if (!line) continue;
    const f = line.split(US);
    if (f.length < 8) continue;
    out.push({
      hash: f[0],
      short: f[1],
      author: f[2],
      email: f[3],
      ts: parseInt(f[4], 10) || 0,
      parents: f[5].trim() ? f[5].trim().split(" ") : [],
      refs: f[6],
      subject: f[7],
    });
  }
  return out;
}

// ── Commit-graph swimlane layout ──────────────────────────────────────────
// Assigns each commit a lane (column) and computes the line segments that
// connect a row to the next, gitk / VS Code style. Pure + deterministic so it
// can be unit-tested and rendered as SVG.

export interface GraphSeg {
  fromCol: number; // lane column at the TOP edge of this row
  toCol: number; // lane column at the BOTTOM edge of this row
  color: number; // palette index
  node: boolean; // true if this segment terminates at the row's commit node
}
export interface GraphRow {
  col: number; // column of the commit node
  color: number;
  segments: GraphSeg[];
  width: number; // lane count spanned by this row (for sizing)
}

export function buildGraph(commits: Commit[]): GraphRow[] {
  const rows: GraphRow[] = [];
  let lanes: (string | null)[] = []; // hash each active lane is waiting for
  const colorOf = new Map<string, number>();
  let nextColor = 0;
  const colorFor = (hash: string): number => {
    let c = colorOf.get(hash);
    if (c === undefined) {
      c = nextColor++;
      colorOf.set(hash, c);
    }
    return c;
  };

  for (const commit of commits) {
    let col = lanes.indexOf(commit.hash);
    if (col === -1) {
      col = lanes.indexOf(null);
      if (col === -1) {
        col = lanes.length;
        lanes.push(null);
      }
      lanes[col] = commit.hash;
    }
    const nodeColor = colorFor(commit.hash);
    const top = lanes.slice();

    // Advance: the node's lane becomes its first parent; extra parents (merge)
    // open new lanes. The first parent inherits this branch's color.
    const parents = commit.parents;
    if (parents.length === 0) {
      lanes[col] = null;
    } else {
      lanes[col] = parents[0];
      if (!colorOf.has(parents[0])) colorOf.set(parents[0], nodeColor);
      for (let p = 1; p < parents.length; p++) {
        let fc = lanes.indexOf(null);
        if (fc === -1) {
          fc = lanes.length;
          lanes.push(null);
        }
        lanes[fc] = parents[p];
        colorFor(parents[p]);
      }
    }
    // Other lanes also waiting for this commit collapse into the node.
    for (let i = 0; i < lanes.length; i++) {
      if (i !== col && top[i] === commit.hash) lanes[i] = null;
    }
    while (lanes.length && lanes[lanes.length - 1] === null) lanes.pop();
    const bottom = lanes.slice();

    const segments: GraphSeg[] = [];
    // Lanes entering at the top: either pass straight through or fold into node.
    for (let i = 0; i < top.length; i++) {
      const t = top[i];
      if (t == null) continue;
      if (t === commit.hash) {
        segments.push({ fromCol: i, toCol: col, color: colorFor(commit.hash), node: true });
      } else {
        const j = bottom.indexOf(t);
        if (j !== -1) segments.push({ fromCol: i, toCol: j, color: colorFor(t), node: false });
      }
    }
    // Node's outgoing edges to each parent's lane at the bottom.
    for (const par of parents) {
      const j = bottom.indexOf(par);
      if (j !== -1) segments.push({ fromCol: col, toCol: j, color: colorFor(par), node: true });
    }

    rows.push({ col, color: nodeColor, segments, width: Math.max(top.length, bottom.length, col + 1) });
  }
  return rows;
}

export interface Conventional {
  kind: string;
  scope: string;
  rest: string;
}

// Recognize `type(scope): rest` / `type: rest`. Returns null otherwise.
export function parseConventional(subject: string): Conventional | null {
  const colon = subject.indexOf(":");
  if (colon < 0) return null;
  let head = subject.slice(0, colon);
  let scope = "";
  const lp = head.indexOf("(");
  if (lp >= 0) {
    const rp = head.indexOf(")");
    if (rp < lp) return null;
    scope = head.slice(lp + 1, rp);
    head = head.slice(0, lp);
  }
  if (!head.length || !/^[a-z]+$/.test(head)) return null;
  return { kind: head, scope, rest: subject.slice(colon + 1).trimStart() };
}

export interface Change {
  code: string; // A M D R ?
  staged: boolean;
  path: string;
}

// Group a flat change list into a collapsible folder tree (Terax-style),
// dirs first then files, each alphabetical. Pure for unit testing.
export interface FileNode {
  name: string;
  path: string; // dir path for folders, file path for leaves
  dir: boolean;
  change?: Change;
  children: FileNode[];
}

export function buildFileTree(changes: Change[]): FileNode[] {
  const root: FileNode = { name: "", path: "", dir: true, children: [] };
  for (const ch of changes) {
    const parts = ch.path.split("/");
    let node = root;
    let acc = "";
    for (let i = 0; i < parts.length; i++) {
      const part = parts[i];
      acc = acc ? `${acc}/${part}` : part;
      const leaf = i === parts.length - 1;
      let next = node.children.find((c) => c.name === part && c.dir === !leaf);
      if (!next) {
        next = { name: part, path: acc, dir: !leaf, children: [], change: leaf ? ch : undefined };
        node.children.push(next);
      }
      node = next;
    }
  }
  const sort = (nodes: FileNode[]) => {
    nodes.sort((a, b) => (a.dir === b.dir ? a.name.localeCompare(b.name) : a.dir ? -1 : 1));
    for (const n of nodes) if (n.dir) sort(n.children);
  };
  sort(root.children);
  return root.children;
}

export function parseStatus(raw: string): { branch: string; changes: Change[] } {
  let branch = "";
  const changes: Change[] = [];
  for (const line of raw.split("\n")) {
    if (line.startsWith("## ")) {
      branch = line.slice(3).split(/\.\.\.| /)[0];
      continue;
    }
    if (line.length < 3) continue;
    const x = line[0];
    const y = line[1];
    const path = line.slice(3).replace(/^ +/, "");
    if (x === "?" && y === "?") changes.push({ code: "?", staged: false, path });
    else if (x !== " " && x !== "?") changes.push({ code: x, staged: true, path });
    else changes.push({ code: y, staged: false, path });
  }
  return { branch, changes };
}

// ── Hunk-level staging (#62) ───────────────────────────────────────────────
// Split a single-file unified diff (`git diff -- path`) into its file header
// and individual hunks, so each can be staged/discarded on its own. Pure so it
// is unit-testable; the apply itself shells `git apply` in Rust.

export interface Hunk {
  header: string; // the `@@ -a,b +c,d @@ ...` line
  body: string; // hunk including the @@ line and its +/-/context lines
}

export interface FileDiff {
  preamble: string; // everything before the first @@ (diff --git, index, ---/+++)
  hunks: Hunk[];
}

export function parseHunks(diff: string): FileDiff {
  const lines = diff.split("\n");
  const preamble: string[] = [];
  const hunks: Hunk[] = [];
  let i = 0;
  while (i < lines.length && !lines[i].startsWith("@@")) {
    preamble.push(lines[i]);
    i++;
  }
  while (i < lines.length) {
    if (!lines[i].startsWith("@@")) break;
    const header = lines[i];
    const body: string[] = [lines[i]];
    i++;
    while (i < lines.length && !lines[i].startsWith("@@")) {
      body.push(lines[i]);
      i++;
    }
    // Drop a trailing empty line that split("\n") adds at EOF.
    while (body.length > 1 && body[body.length - 1] === "") body.pop();
    hunks.push({ header, body: body.join("\n") });
  }
  return { preamble: preamble.join("\n"), hunks };
}

// Reconstruct a standalone, applyable patch for one hunk: the file preamble
// plus just that hunk. Always ends with a trailing newline (git apply needs it).
export function buildHunkPatch(file: FileDiff, index: number): string {
  const h = file.hunks[index];
  if (!h) return "";
  return `${file.preamble}\n${h.body}\n`;
}

// Stage-by-line (#22): build a patch for a hunk that includes only the selected
// changed lines. `selected` holds body-line indices (1-based against the hunk
// body, matching the array index in body.split("\n")). Unselected `-` lines
// become context (the deletion isn't staged); unselected `+` lines are dropped.
// The `@@` header line-counts are recomputed so `git apply --cached` accepts it.
export function buildLinePatch(file: FileDiff, index: number, selected: Set<number>): string {
  const h = file.hunks[index];
  if (!h) return "";
  const lines = h.body.split("\n");
  const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(lines[0]);
  if (!m) return "";
  const oldStart = Number(m[1]);
  const out: string[] = [];
  let oldCount = 0;
  let newCount = 0;
  for (let i = 1; i < lines.length; i++) {
    const l = lines[i];
    if (l === "") continue;
    const k = l[0];
    if (k === " ") { out.push(l); oldCount++; newCount++; }
    else if (k === "-") {
      if (selected.has(i)) { out.push(l); oldCount++; }
      else { out.push(" " + l.slice(1)); oldCount++; newCount++; }
    } else if (k === "+") {
      if (selected.has(i)) { out.push(l); newCount++; }
    } else { out.push(l); }
  }
  if (!out.some((l) => l[0] === "+" || l[0] === "-")) return ""; // nothing staged
  const header = `@@ -${oldStart},${oldCount} +${oldStart},${newCount} @@`;
  return `${file.preamble}\n${header}\n${out.join("\n")}\n`;
}

// Relative "2m" / "3h" / "5d" / "Jan 4" style timestamp from unix seconds.
export function relTime(ts: number, nowMs: number): string {
  const s = Math.max(0, Math.floor(nowMs / 1000) - ts);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  if (s < 86400 * 30) return `${Math.floor(s / 86400)}d`;
  const d = new Date(ts * 1000);
  return d.toLocaleString("en-US", { month: "short", day: "numeric" });
}
