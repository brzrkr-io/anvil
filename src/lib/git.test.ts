import { describe, it, expect } from "vitest";
import { parseLog, parseConventional, parseStatus, relTime, buildGraph, buildFileTree, parseHunks, buildHunkPatch, buildLinePatch, type Commit, type Change } from "./git.js";

const US = "\x1f";

function commit(hash: string, parents: string[]): Commit {
  return { hash, short: hash, author: "a", email: "e", ts: 0, parents, refs: "", subject: hash };
}

describe("buildFileTree", () => {
  const ch = (path: string): Change => ({ code: "M", staged: false, path });

  it("nests files under folders, dirs before files", () => {
    const tree = buildFileTree([ch("src/app.ts"), ch("README.md"), ch("src/lib/git.ts")]);
    expect(tree.map((n) => n.name)).toEqual(["src", "README.md"]); // dir first
    const src = tree[0];
    expect(src.dir).toBe(true);
    expect(src.children.map((n) => n.name)).toEqual(["lib", "app.ts"]); // nested dir before file
    expect(src.children[0].children[0].path).toBe("src/lib/git.ts");
  });

  it("attaches the change to leaf nodes only", () => {
    const tree = buildFileTree([ch("a/b.ts")]);
    expect(tree[0].change).toBeUndefined();
    expect(tree[0].children[0].change?.path).toBe("a/b.ts");
  });
});

describe("buildGraph", () => {
  it("keeps linear history in a single lane", () => {
    const rows = buildGraph([commit("A", ["B"]), commit("B", ["C"]), commit("C", [])]);
    expect(rows.map((r) => r.col)).toEqual([0, 0, 0]);
    expect(Math.max(...rows.map((r) => r.width))).toBe(1);
  });

  it("opens a second lane for a merge commit", () => {
    const rows = buildGraph([
      commit("M", ["P1", "P2"]),
      commit("P1", ["B"]),
      commit("P2", ["B"]),
      commit("B", []),
    ]);
    // merge row must span two lanes and route one edge to column 1
    expect(rows[0].width).toBeGreaterThanOrEqual(2);
    expect(rows[0].segments.some((s) => s.toCol === 1)).toBe(true);
    // the two branches collapse back to a single lane at B
    expect(rows[3].col).toBe(0);
  });

  it("gives diverging branches distinct colors", () => {
    const rows = buildGraph([commit("M", ["P1", "P2"]), commit("P1", ["B"]), commit("P2", ["B"]), commit("B", [])]);
    const p1 = rows[1].color;
    const p2 = rows[2].color;
    expect(p1).not.toBe(p2);
  });
});

describe("parseLog", () => {
  it("parses a single commit line into a Commit record", () => {
    const hash = "abc123def456abc123def456abc123def456abc1";
    const short = "abc123d";
    const line = [hash, short, "Alice", "alice@example.com", "1700000000", "parent1 parent2", "HEAD -> main", "fix: typo"].join(US);
    const commits = parseLog(line);
    expect(commits).toHaveLength(1);
    const c = commits[0];
    expect(c.hash).toBe(hash);
    expect(c.short).toBe(short);
    expect(c.author).toBe("Alice");
    expect(c.email).toBe("alice@example.com");
    expect(c.ts).toBe(1700000000);
    expect(c.parents).toEqual(["parent1", "parent2"]);
    expect(c.refs).toBe("HEAD -> main");
    expect(c.subject).toBe("fix: typo");
  });

  it("parses multiple commits separated by newlines", () => {
    const line1 = ["aaa", "aaa", "Bob", "b@x.com", "1000", "", "", "feat: a"].join(US);
    const line2 = ["bbb", "bbb", "Carol", "c@x.com", "2000", "aaa", "", "chore: b"].join(US);
    const commits = parseLog(line1 + "\n" + line2);
    expect(commits).toHaveLength(2);
    expect(commits[0].hash).toBe("aaa");
    expect(commits[1].hash).toBe("bbb");
  });

  it("skips empty lines", () => {
    const line = ["h", "s", "A", "a@b.com", "0", "", "", "msg"].join(US);
    expect(parseLog("\n" + line + "\n\n")).toHaveLength(1);
  });

  it("skips lines with fewer than 8 fields", () => {
    const short = ["h", "s", "A"].join(US);
    expect(parseLog(short)).toHaveLength(0);
  });

  it("parses a single parent", () => {
    const line = ["h", "s", "A", "a@b.com", "0", "parentX", "", "msg"].join(US);
    expect(parseLog(line)[0].parents).toEqual(["parentX"]);
  });

  it("parses empty parents as empty array", () => {
    const line = ["h", "s", "A", "a@b.com", "0", "", "", "msg"].join(US);
    expect(parseLog(line)[0].parents).toEqual([]);
  });

  it("defaults ts to 0 for non-numeric timestamp", () => {
    const line = ["h", "s", "A", "a@b.com", "not-a-number", "", "", "msg"].join(US);
    expect(parseLog(line)[0].ts).toBe(0);
  });
});

describe("parseConventional", () => {
  it("parses type: rest (no scope)", () => {
    const r = parseConventional("fix: correct null check");
    expect(r).not.toBeNull();
    expect(r!.kind).toBe("fix");
    expect(r!.scope).toBe("");
    expect(r!.rest).toBe("correct null check");
  });

  it("parses type(scope): rest", () => {
    const r = parseConventional("fix(sc): x");
    expect(r).not.toBeNull();
    expect(r!.kind).toBe("fix");
    expect(r!.scope).toBe("sc");
    expect(r!.rest).toBe("x");
  });

  it("returns null for Merge commits (uppercase type)", () => {
    expect(parseConventional("Merge branch 'main' into dev")).toBeNull();
  });

  it("returns null when no colon present", () => {
    expect(parseConventional("just a plain message")).toBeNull();
  });

  it("returns null for uppercase type", () => {
    expect(parseConventional("Fix: something")).toBeNull();
  });

  it("trims leading whitespace from rest", () => {
    const r = parseConventional("feat:   add thing");
    expect(r!.rest).toBe("add thing");
  });

  it("returns null when closing paren precedes opening paren", () => {
    expect(parseConventional("fix)(scope: bad")).toBeNull();
  });
});

describe("parseStatus", () => {
  const sample = [
    "## main...origin/main",
    " M src/app.ts",
    "?? new-file.txt",
    "A  staged.ts",
    "M  also-staged.ts",
    "",
  ].join("\n");

  it("extracts branch name", () => {
    const { branch } = parseStatus(sample);
    expect(branch).toBe("main");
  });

  it("classifies untracked files as unstaged code '?'", () => {
    const { changes } = parseStatus(sample);
    const untracked = changes.find((c) => c.path === "new-file.txt");
    expect(untracked).toBeDefined();
    expect(untracked!.staged).toBe(false);
    expect(untracked!.code).toBe("?");
  });

  it("classifies ' M' as unstaged modification", () => {
    const { changes } = parseStatus(sample);
    const modified = changes.find((c) => c.path === "src/app.ts");
    expect(modified).toBeDefined();
    expect(modified!.staged).toBe(false);
    expect(modified!.code).toBe("M");
  });

  it("classifies 'A ' as staged addition", () => {
    const { changes } = parseStatus(sample);
    const staged = changes.find((c) => c.path === "staged.ts");
    expect(staged).toBeDefined();
    expect(staged!.staged).toBe(true);
    expect(staged!.code).toBe("A");
  });

  it("classifies 'M ' as staged modification", () => {
    const { changes } = parseStatus(sample);
    const staged = changes.find((c) => c.path === "also-staged.ts");
    expect(staged).toBeDefined();
    expect(staged!.staged).toBe(true);
    expect(staged!.code).toBe("M");
  });

  it("handles detached HEAD (no remote tracking)", () => {
    const { branch } = parseStatus("## HEAD (no branch)\n");
    expect(branch).toBe("HEAD");
  });

  it("returns empty changes for empty input", () => {
    const { branch, changes } = parseStatus("");
    expect(branch).toBe("");
    expect(changes).toHaveLength(0);
  });
});

describe("relTime", () => {
  const now = 1700000000 * 1000; // nowMs

  it("returns seconds for < 60s ago", () => {
    expect(relTime(1700000000 - 30, now)).toBe("30s");
  });

  it("returns minutes for 60s–3599s ago", () => {
    expect(relTime(1700000000 - 120, now)).toBe("2m");
  });

  it("returns hours for 3600s–86399s ago", () => {
    expect(relTime(1700000000 - 7200, now)).toBe("2h");
  });

  it("returns days for 1–29 days ago", () => {
    expect(relTime(1700000000 - 86400 * 3, now)).toBe("3d");
  });

  it("returns a formatted date string for >= 30 days ago", () => {
    // 31 days ago
    const ts = 1700000000 - 86400 * 31;
    const result = relTime(ts, now);
    // Should be a short-month + day string, e.g. "Oct 3"
    expect(result).toMatch(/^[A-Z][a-z]+ \d+$/);
  });

  it("clamps negative delta to 0s", () => {
    // ts is in the future relative to now
    expect(relTime(1700000000 + 100, now)).toBe("0s");
  });
});

describe("parseHunks / buildHunkPatch", () => {
  const diff = [
    "diff --git a/foo.txt b/foo.txt",
    "index 1111111..2222222 100644",
    "--- a/foo.txt",
    "+++ b/foo.txt",
    "@@ -1,3 +1,4 @@ ctx one",
    " line1",
    "-old",
    "+new",
    "+added",
    " line3",
    "@@ -10,2 +11,2 @@ ctx two",
    " keep",
    "-drop",
    "+swap",
    "",
  ].join("\n");

  it("separates the file preamble from the hunks", () => {
    const f = parseHunks(diff);
    expect(f.preamble).toBe(
      "diff --git a/foo.txt b/foo.txt\nindex 1111111..2222222 100644\n--- a/foo.txt\n+++ b/foo.txt",
    );
    expect(f.hunks).toHaveLength(2);
  });

  it("captures each hunk body starting at its @@ header", () => {
    const f = parseHunks(diff);
    expect(f.hunks[0].header).toBe("@@ -1,3 +1,4 @@ ctx one");
    expect(f.hunks[0].body).toBe("@@ -1,3 +1,4 @@ ctx one\n line1\n-old\n+new\n+added\n line3");
    expect(f.hunks[1].body).toBe("@@ -10,2 +11,2 @@ ctx two\n keep\n-drop\n+swap");
  });

  it("builds an applyable single-hunk patch (preamble + one hunk + trailing newline)", () => {
    const f = parseHunks(diff);
    const patch = buildHunkPatch(f, 1);
    expect(patch).toBe(
      "diff --git a/foo.txt b/foo.txt\nindex 1111111..2222222 100644\n--- a/foo.txt\n+++ b/foo.txt\n" +
        "@@ -10,2 +11,2 @@ ctx two\n keep\n-drop\n+swap\n",
    );
    expect(patch.endsWith("\n")).toBe(true);
  });

  it("returns empty string for an out-of-range hunk index", () => {
    const f = parseHunks(diff);
    expect(buildHunkPatch(f, 9)).toBe("");
  });

  it("handles a diff with no hunks (empty)", () => {
    const f = parseHunks("");
    expect(f.hunks).toHaveLength(0);
  });

  it("stages a single added line, dropping the other addition and keeping deletion as context", () => {
    const f = parseHunks(diff);
    // hunk 0 body indices: 1=' line1' 2='-old' 3='+new' 4='+added' 5=' line3'
    const patch = buildLinePatch(f, 0, new Set([3]));
    expect(patch).toBe(
      "diff --git a/foo.txt b/foo.txt\nindex 1111111..2222222 100644\n--- a/foo.txt\n+++ b/foo.txt\n" +
        "@@ -1,3 +1,4 @@\n line1\n old\n+new\n line3\n",
    );
  });

  it("stages a single deletion, dropping unselected additions", () => {
    const f = parseHunks(diff);
    const patch = buildLinePatch(f, 0, new Set([2]));
    expect(patch).toBe(
      "diff --git a/foo.txt b/foo.txt\nindex 1111111..2222222 100644\n--- a/foo.txt\n+++ b/foo.txt\n" +
        "@@ -1,3 +1,2 @@\n line1\n-old\n line3\n",
    );
  });

  it("returns empty when no changed lines are selected", () => {
    const f = parseHunks(diff);
    expect(buildLinePatch(f, 0, new Set([1]))).toBe(""); // only a context line
  });
});
