// Pure parsing for the Docker containers view (roadmap I81). Parses
// `docker ps --format {{json .}}` (newline-delimited JSON) into sorted rows.

export type ContainerState = "running" | "exited" | "paused" | "other";

export interface Container {
  id: string;
  name: string;
  image: string;
  state: ContainerState;
  status: string;
  ports: string;
}

interface RawPs {
  ID?: string;
  Names?: string;
  Image?: string;
  State?: string;
  Status?: string;
  Ports?: string;
}

// Derive a normalized state. Newer docker emits a State field; fall back to
// parsing the Status string ("Up 3 minutes" / "Exited (0) …") for older CLIs.
export function containerState(r: Pick<RawPs, "State" | "Status">): ContainerState {
  const s = (r.State ?? "").toLowerCase();
  if (s === "running" || s === "exited" || s === "paused") return s;
  const st = (r.Status ?? "").toLowerCase();
  if (st.startsWith("up")) return st.includes("paused") ? "paused" : "running";
  if (st.startsWith("exited")) return "exited";
  return "other";
}

const RANK: Record<ContainerState, number> = { running: 0, paused: 1, other: 2, exited: 3 };

export function parseContainers(raw: string): Container[] {
  const rows: Container[] = [];
  for (const line of raw.split("\n")) {
    const t = line.trim();
    if (!t) continue;
    let j: RawPs;
    try {
      j = JSON.parse(t);
    } catch {
      continue;
    }
    if (!j.ID) continue;
    rows.push({
      id: j.ID.slice(0, 12),
      name: j.Names ?? j.ID.slice(0, 12),
      image: j.Image ?? "",
      state: containerState(j),
      status: j.Status ?? "",
      ports: j.Ports ?? "",
    });
  }
  return rows.sort((a, b) => RANK[a.state] - RANK[b.state] || a.name.localeCompare(b.name));
}

export function runningCount(items: Pick<Container, "state">[]): number {
  return items.filter((c) => c.state === "running").length;
}
