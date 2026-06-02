<script lang="ts">
  // "What needs attention" — one list of everything broken across the beat-target
  // loops: failing Flux reconciles, broken pods, red CI runs. Each item jumps to
  // its surface or hands the agent a focused investigation. Read-only scan.
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import { parseFluxItems, oneLine } from "$lib/flux-health";
  import { parseRuns } from "$lib/actions-runs";
  import { fluxInvestigation, actionsInvestigation } from "$lib/agent-ops";

  let { cwd = "", onClose, onOpenView, onInvestigate }:
    { cwd?: string; onClose: () => void; onOpenView?: (view: string) => void; onInvestigate?: (prompt: string) => void } = $props();

  interface Item {
    group: "GitOps" | "Workloads" | "CI";
    icon: string;
    label: string;
    detail: string;
    view: string;
    investigate?: string;
  }
  let items = $state<Item[]>([]);
  let loading = $state(true);

  const BROKEN_POD = /Error|CrashLoop|Failed|Evicted|ImagePull|Pending|Unknown|Init:|Terminating|OOMKilled/i;

  function parsePods(raw: string): Item[] {
    const lines = raw.split("\n").filter(Boolean);
    if (!lines.length || !/^NAMESPACE\s/.test(lines[0])) return [];
    return lines.slice(1)
      .map((l) => l.split(/\s+/))
      .filter((c) => c[1] && BROKEN_POD.test(c[3] ?? ""))
      .map((c) => ({
        group: "Workloads" as const,
        icon: "kube",
        label: `${c[0]}/${c[1]}`,
        detail: `${c[3]}${Number(c[4]) > 0 ? ` · ${c[4]} restarts` : ""}`,
        view: "k8s",
      }));
  }

  async function scan() {
    loading = true;
    const [ks, hr, pods, runs] = await Promise.allSettled([
      invoke<string>("flux_get", { kind: "kustomizations" }),
      invoke<string>("flux_get", { kind: "helmreleases" }),
      invoke<string>("kube_pods", { context: "" }),
      invoke<string>("gh_runs_json", { cwd }),
    ]);
    const out: Item[] = [];
    for (const r of [ks, hr]) {
      if (r.status !== "fulfilled") continue;
      for (const f of parseFluxItems(r.value).filter((x) => x.ready === "fail")) {
        out.push({
          group: "GitOps",
          icon: "flux",
          label: `${f.apiKind} ${f.ns}/${f.name}`,
          detail: oneLine(f.message) || "not ready",
          view: "k8s",
          investigate: fluxInvestigation(f.apiKind ?? "Kustomization", f.name, f.ns, f.message),
        });
      }
    }
    if (pods.status === "fulfilled") out.push(...parsePods(pods.value));
    if (runs.status === "fulfilled") {
      for (const run of parseRuns(runs.value).filter((x) => x.state === "fail")) {
        out.push({
          group: "CI",
          icon: "ci",
          label: run.title || run.workflow || run.id,
          detail: [run.workflow, run.branch].filter(Boolean).join(" · "),
          view: "ci",
          investigate: actionsInvestigation(run.id, run.workflow, run.branch),
        });
      }
    }
    items = out;
    loading = false;
  }
  scan();

  const groups = $derived(["GitOps", "Workloads", "CI"].map((g) => ({ g, rows: items.filter((i) => i.group === g) })).filter((x) => x.rows.length));

  function open(it: Item) { onOpenView?.(it.view); onClose(); }
  function investigate(it: Item) { if (it.investigate) { onInvestigate?.(it.investigate); onClose(); } }
</script>

<div class="at-scrim" onclick={onClose} role="presentation"></div>
<div class="at" role="dialog" aria-label="What needs attention">
  <div class="at-head">
    <div class="at-titles">
      <span class="at-title">Needs attention</span>
      <span class="at-sub">{loading ? "scanning…" : items.length ? `${items.length} across GitOps · Workloads · CI` : "all clear"}</span>
    </div>
    <span class="spacer"></span>
    <button class="at-btn" onclick={scan} disabled={loading}>{loading ? "…" : "Rescan"}</button>
    <button class="at-x" onclick={onClose} title="Close"><Icon name="close" size={13} /></button>
  </div>

  <div class="at-list">
    {#if !loading && !items.length}
      <div class="at-clear"><Icon name="check" size={22} /><span>Nothing failing. GitOps green, pods healthy, CI passing.</span></div>
    {/if}
    {#each groups as grp (grp.g)}
      <div class="at-group">{grp.g}</div>
      {#each grp.rows as it (it.group + it.label)}
        <div class="at-row">
          <span class="at-dot"></span>
          <span class="at-ic"><Icon name={it.icon} size={14} /></span>
          <div class="at-text">
            <span class="at-name">{it.label}</span>
            <span class="at-detail" title={it.detail}>{it.detail}</span>
          </div>
          <span class="spacer"></span>
          {#if it.investigate}<button class="at-act ai" onclick={() => investigate(it)} title="Investigate with the agent"><Icon name="agent" size={12} /></button>{/if}
          <button class="at-act" onclick={() => open(it)}>Open</button>
        </div>
      {/each}
    {/each}
  </div>
</div>

<style>
  .at-scrim { position: fixed; inset: 0; z-index: 90; background: var(--glass-scrim, rgba(0,0,0,0.4));
    backdrop-filter: blur(4px); -webkit-backdrop-filter: blur(4px); }
  .at { position: fixed; z-index: 91; top: 50%; left: 50%; transform: translate(-50%, -50%);
    width: 560px; max-width: 92vw; max-height: 80vh; display: flex; flex-direction: column;
    background: var(--panel); border: 1px solid var(--border); border-radius: 10px;
    box-shadow: var(--elev-3, 0 12px 40px rgba(0,0,0,0.45)); font-family: var(--font-ui); overflow: hidden; }
  .at-head { display: flex; align-items: center; gap: 10px; padding: 14px 16px 12px; border-bottom: 1px solid var(--hairline); }
  .at-titles { display: flex; flex-direction: column; gap: 2px; }
  .at-title { font-size: 15px; font-weight: 600; color: var(--text); }
  .at-sub { font-size: 11px; color: var(--text3); }
  .spacer { flex: 1; }
  .at-btn { font-size: 11.5px; color: var(--text2); background: var(--panel2); border: 1px solid var(--border);
    border-radius: 6px; padding: 4px 10px; cursor: default; font-family: var(--font-ui); }
  .at-btn:hover:not(:disabled) { color: var(--text); border-color: var(--accent); }
  .at-btn:disabled { opacity: 0.5; }
  .at-x { background: none; border: 0; color: var(--text3); cursor: default; display: inline-flex; }
  .at-x:hover { color: var(--text); }
  .at-list { flex: 1; min-height: 0; overflow-y: auto; padding: 6px 8px 10px; }
  .at-group { font-size: 10px; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text3);
    padding: 10px 8px 4px; }
  .at-row { display: flex; align-items: center; gap: 9px; padding: 8px; border-radius: 7px; }
  .at-row:hover { background: color-mix(in srgb, var(--text) 4%, transparent); }
  .at-dot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; background: var(--status-failure, #e5484d); }
  .at-ic { color: var(--text2); flex: 0 0 auto; display: inline-flex; }
  .at-text { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .at-name { font-size: 12.5px; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 320px; }
  .at-detail { font-size: 11px; color: var(--text3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 320px; }
  .at-act { font-size: 11.5px; color: var(--text2); background: var(--panel2); border: 1px solid var(--border);
    border-radius: 6px; padding: 3px 10px; cursor: default; flex: 0 0 auto; font-family: var(--font-ui); }
  .at-act:hover { color: var(--text); border-color: var(--accent); }
  .at-act.ai { color: var(--status-agent, #a371f7); border-color: transparent; background: transparent; padding: 3px 6px; }
  .at-clear { display: flex; flex-direction: column; align-items: center; gap: 10px; padding: 36px 20px;
    color: var(--status-verified, var(--green, #3fb950)); text-align: center; }
  .at-clear span { color: var(--text3); font-size: 12px; }
</style>
