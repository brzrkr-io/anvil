// Pure git merge-conflict parser (#34). Handles both the default 2-way style
// (<<<<<<< / ======= / >>>>>>>) and diff3 (adds a ||||||| base section), so the
// editor can offer true 3-way resolution: take ours, theirs, base, or both.
// Line indices are 0-based into the file's line array.

export type MergeChoice = "ours" | "theirs" | "base" | "both";

export interface Conflict {
  start: number; // index of the <<<<<<< line
  end: number; // index of the >>>>>>> line
  ours: string[]; // lines between <<<<<<< and (||||||| or =======)
  base: string[]; // lines between ||||||| and ======= (empty if not diff3)
  theirs: string[]; // lines between ======= and >>>>>>>
  oursLabel: string; // text after <<<<<<<
  theirsLabel: string; // text after >>>>>>>
}

export function parseConflicts(lines: string[]): Conflict[] {
  const out: Conflict[] = [];
  let i = 0;
  while (i < lines.length) {
    if (!lines[i].startsWith("<<<<<<<")) {
      i++;
      continue;
    }
    const start = i;
    const oursLabel = lines[i].slice(7).trim();
    const ours: string[] = [];
    const base: string[] = [];
    const theirs: string[] = [];
    let baseSep = -1;
    let mid = -1;
    let end = -1;
    let j = i + 1;
    for (; j < lines.length; j++) {
      const l = lines[j];
      if (l.startsWith("|||||||") && baseSep < 0 && mid < 0) baseSep = j;
      else if (l.startsWith("=======") && mid < 0) mid = j;
      else if (l.startsWith(">>>>>>>")) {
        end = j;
        break;
      }
    }
    if (mid < 0 || end < 0) {
      // Malformed/unterminated: skip past this marker.
      i = start + 1;
      continue;
    }
    const oursEnd = baseSep >= 0 ? baseSep : mid;
    for (let k = start + 1; k < oursEnd; k++) ours.push(lines[k]);
    if (baseSep >= 0) for (let k = baseSep + 1; k < mid; k++) base.push(lines[k]);
    for (let k = mid + 1; k < end; k++) theirs.push(lines[k]);
    out.push({
      start,
      end,
      ours,
      base,
      theirs,
      oursLabel,
      theirsLabel: lines[end].slice(7).trim(),
    });
    i = end + 1;
  }
  return out;
}

// The replacement lines a given choice produces for one conflict's span.
export function resolvedLines(c: Conflict, choice: MergeChoice): string[] {
  switch (choice) {
    case "ours":
      return c.ours;
    case "theirs":
      return c.theirs;
    case "base":
      return c.base;
    case "both":
      return [...c.ours, ...c.theirs];
  }
}

// Resolve every conflict in a file with the same choice; returns new file lines.
export function resolveAll(lines: string[], choice: MergeChoice): string[] {
  const conflicts = parseConflicts(lines);
  if (!conflicts.length) return lines;
  const out: string[] = [];
  let cursor = 0;
  for (const c of conflicts) {
    for (let k = cursor; k < c.start; k++) out.push(lines[k]);
    out.push(...resolvedLines(c, choice));
    cursor = c.end + 1;
  }
  for (let k = cursor; k < lines.length; k++) out.push(lines[k]);
  return out;
}
