// Multi-buffer search-and-edit-in-place (#38). Search results are editable;
// saving rewrites each touched line back into its file. Pure + testable: given a
// file's content and the set of line edits for it, produce the new content.
// Line numbers are 1-based (as grep reports them). Edits replace whole lines and
// never insert/remove lines, so line numbers stay stable across a batch.

export interface LineEdit {
  line: number; // 1-based
  text: string; // new full-line content
}

export function applyLineEdits(content: string, edits: LineEdit[]): string {
  const lines = content.split("\n");
  for (const e of edits) {
    if (e.line >= 1 && e.line <= lines.length) lines[e.line - 1] = e.text;
  }
  return lines.join("\n");
}

// Group a flat edit list (with paths) into per-file edit batches.
export interface PathLineEdit extends LineEdit {
  path: string;
}
export function groupByFile(edits: PathLineEdit[]): Map<string, LineEdit[]> {
  const out = new Map<string, LineEdit[]>();
  for (const e of edits) {
    const arr = out.get(e.path) ?? [];
    arr.push({ line: e.line, text: e.text });
    out.set(e.path, arr);
  }
  return out;
}
