// Subsequence fuzzy match + ranking. Lower score = better (earlier, tighter).
export function score(text: string, query: string): number | null {
  if (!query) return 0;
  const t = text.toLowerCase();
  const q = query.toLowerCase();
  let ti = 0;
  let qi = 0;
  let s = 0;
  let last = -1;
  while (ti < t.length && qi < q.length) {
    if (t[ti] === q[qi]) {
      s += last < 0 ? ti : ti - last - 1; // position + gap penalty
      last = ti;
      qi += 1;
    }
    ti += 1;
  }
  return qi === q.length ? s : null;
}

export interface Ranked<T> { item: T; s: number; }

export function rank<T>(items: T[], query: string, key: (t: T) => string, limit = 200): T[] {
  const out: Ranked<T>[] = [];
  for (const item of items) {
    const sc = score(key(item), query);
    if (sc !== null) out.push({ item, s: sc });
    if (!query && out.length >= limit) break;
  }
  out.sort((a, b) => a.s - b.s);
  return out.slice(0, limit).map((r) => r.item);
}
