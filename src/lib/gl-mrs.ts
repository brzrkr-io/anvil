// Parse `glab api …/merge_requests` JSON into compact rows. (#46)

export interface MrRow {
  iid: string;
  title: string;
  source: string; // source branch
  target: string; // target branch
  url: string; // web_url
  draft: boolean;
}

interface RawMr {
  iid?: number;
  id?: number;
  title?: string;
  source_branch?: string;
  target_branch?: string;
  web_url?: string;
  draft?: boolean;
  work_in_progress?: boolean;
}

export function parseMrRows(raw: string): MrRow[] {
  let j: unknown;
  try {
    j = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(j)) return [];
  return (j as RawMr[])
    .map((m) => ({
      iid: String(m.iid ?? m.id ?? ""),
      title: m.title ?? "",
      source: m.source_branch ?? "",
      target: m.target_branch ?? "",
      url: m.web_url ?? "",
      draft: m.draft === true || m.work_in_progress === true || /^draft:|^wip:/i.test(m.title ?? ""),
    }))
    .filter((r) => r.iid)
    .sort((a, b) => Number(a.iid) - Number(b.iid));
}
