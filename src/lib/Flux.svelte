<script lang="ts">
  import { onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import Skeleton from "$lib/Skeleton.svelte";
  import EmptyState from "$lib/EmptyState.svelte";
  import { toast } from "$lib/toast";
  import { askConfirm } from "$lib/dialog";
  import { failingCount, oneLine, shortRev } from "$lib/flux-health";
  import { fluxInvestigation } from "$lib/agent-ops";

  let { onRunCommand, onPresence, onHealth, onInvestigate, active = true, visible = true }:
    { onRunCommand?: (cmd: string) => void; onPresence?: (present: boolean) => void; onHealth?: (failing: number) => void; onInvestigate?: (prompt: string) => void;
      // active = the Kubernetes view is open; visible = the Flux list is the shown
      // sub-view. Polling is gated on these so a backgrounded panel does no work.
      active?: boolean; visible?: boolean } = $props();

  type Tab = "kustomizations" | "helmreleases" | "sources" | "images";
  interface FluxItem {
    name: string;
    ns: string;
    apiKind: string; // Kustomization | HelmRelease | GitRepository | ...
    ready: "ok" | "fail" | "unknown";
    suspended: boolean;
    revision: string;
    message: string;
    source: string; // sourceRef name (which Git/OCI/Helm source it reconciles from)
    deps: number;   // count of dependsOn entries
    dependsOn?: string[]; // names of the objects this one waits on (#29)
  }

  let tab = $state<Tab>("kustomizations");
  let items = $state<FluxItem[]>([]);
  let loading = $state(false);
  let err = $state("");
  let present = $state(true);     // Flux installed (core CRDs exist) → show the panel + tabs
  let tabPresent = $state(true);  // the ACTIVE tab's CRD exists → only gates the body
  let busyRow = $state("");

  const TABS: { id: Tab; label: string }[] = [
    { id: "kustomizations", label: "Kustomizations" },
    { id: "helmreleases", label: "HelmReleases" },
    { id: "sources", label: "Sources" },
    { id: "images", label: "Images" },
  ];

  // Map the k8s object kind → the `flux` CLI kind argument the backend allow-lists.
  function fluxKind(apiKind: string): string {
    switch (apiKind) {
      case "Kustomization": return "kustomization";
      case "HelmRelease": return "helmrelease";
      case "GitRepository": return "source git";
      case "OCIRepository": return "source oci";
      case "HelmRepository": return "source helm";
      case "HelmChart": return "source chart";
      case "Bucket": return "source bucket";
      default: return "";
    }
  }

  // Data layer: the backend watcher (watch.rs) fetches + parses + sorts each Flux
  // CRD in Rust off the UI thread and pushes shaped rows over kube://flux:<tab>.
  // The frontend subscribes — no polling, no JSON parse here.
  interface FluxListPayload { rows: FluxItem[]; present: boolean; error: string }

  let unlistenList: (() => void) | undefined;
  let watchedTab = "";

  // One-shot shaped snapshot (instant + CRD-presence probe, even when hidden).
  async function refreshList(t: Tab = tab) {
    try {
      const p = await invoke<FluxListPayload>("kube_snapshot", { kind: `flux:${t}` });
      if (t !== tab) return;
      // Per-tab presence only — a missing Images/Sources CRD must NOT hide the
      // whole Flux panel (that's `present`, owned by the health watcher).
      tabPresent = p.present; items = Array.isArray(p.rows) ? p.rows : []; err = p.error ?? ""; loading = false;
    } catch (e) { err = String(e); loading = false; }
  }

  async function subscribeTab(t: Tab) {
    if (watchedTab === t) return;
    if (watchedTab) invoke("kube_watch_stop", { kind: `flux:${watchedTab}` }).catch(() => {});
    unlistenList?.();
    watchedTab = t;
    loading = true;
    try {
      unlistenList = await listen<FluxListPayload>(`kube://flux:${t}`, (e) => {
        if (watchedTab !== t) return;
        tabPresent = e.payload?.present ?? true; items = Array.isArray(e.payload?.rows) ? e.payload.rows : []; err = e.payload?.error ?? "";
        loading = false;
      });
    } catch { /* no Tauri event bus (browser preview) */ }
    invoke("kube_watch_start", { kind: `flux:${t}`, intervalMs: 6000 }).catch(() => {});
  }
  function stopListWatch() {
    if (watchedTab) { invoke("kube_watch_stop", { kind: `flux:${watchedTab}` }).catch(() => {}); watchedTab = ""; }
    unlistenList?.(); unlistenList = undefined;
  }

  async function act(it: FluxItem, cmd: "flux_reconcile" | "flux_suspend" | "flux_resume", withSource = false) {
    const kind = fluxKind(it.apiKind);
    if (!kind) return;
    if (cmd === "flux_suspend" && !(await askConfirm({ title: "Suspend reconciliation?", message: `${it.apiKind} ${it.ns}/${it.name} will stop reconciling until resumed.`, okLabel: "Suspend", danger: true }))) return;
    busyRow = `${it.ns}/${it.name}`;
    try {
      const args: Record<string, unknown> = { kind, name: it.name, namespace: it.ns };
      if (cmd === "flux_reconcile") args.withSource = withSource;
      const out = await invoke<string>(cmd, args);
      const verb = cmd.replace("flux_", "");
      toast(`${verb} ${it.name}: ${out.trim().split("\n").pop() || "done"}`.slice(0, 120), /error|fail/i.test(out) ? "error" : "success");
      refreshList();
      refreshHealth();
    } catch (e) {
      toast(String(e).slice(0, 160), "error");
    } finally {
      busyRow = "";
    }
  }

  function logs(it: FluxItem) {
    onRunCommand?.(`flux logs --kind=${it.apiKind} --name=${it.name} -n ${it.ns} -f`);
  }

  // Fastest "why" for a failing object: its reconcile events.
  function events(it: FluxItem) {
    onRunCommand?.(`flux events --for ${it.apiKind}/${it.name} -n ${it.ns}`);
  }

  // A2: client-side namespace filter (read-only; no extra cluster calls).
  let nsFilter = $state("");
  // sources + image-automation CRDs are read-only listings (no reconcile/suspend).
  const readonly = $derived(tab === "sources" || tab === "images");
  const namespaces = $derived([...new Set(items.map((i) => i.ns).filter(Boolean))].sort());
  const shown = $derived(nsFilter ? items.filter((i) => i.ns === nsFilter) : items);

  let fails = $derived(failingCount(shown)); // shown set (drives the in-panel chip)

  // A1: cluster-wide failing count (Kustomizations + HelmReleases) for the rail
  // badge — streamed from the backend `flux:health` watcher, regardless of tab.
  let clusterFails = $state(0);
  let unlistenHealth: (() => void) | undefined;
  let healthOn = false;
  // The health watcher is the single source of truth for "is Flux installed":
  // it sets `present` (panel visibility) + notifies the parent, so a per-tab CRD
  // gap can never collapse the panel.
  function applyHealth(h: { failing: number; present: boolean }) {
    clusterFails = h.failing;
    present = h.present;
    onPresence?.(present);
  }
  async function refreshHealth() {
    try { applyHealth(await invoke<{ failing: number; present: boolean }>("kube_snapshot", { kind: "flux:health" })); }
    catch { /* ignore */ }
  }
  async function startHealth() {
    if (healthOn) return;
    healthOn = true;
    try {
      unlistenHealth = await listen<{ failing: number; present: boolean }>("kube://flux:health", (e) => applyHealth(e.payload));
    } catch { /* no event bus */ }
    invoke("kube_watch_start", { kind: "flux:health", intervalMs: 18000 }).catch(() => {});
    refreshHealth();
  }
  function stopHealth() {
    if (!healthOn) return;
    healthOn = false;
    invoke("kube_watch_stop", { kind: "flux:health" }).catch(() => {});
    unlistenHealth?.(); unlistenHealth = undefined;
  }
  $effect(() => { onHealth?.(present ? clusterFails : 0); });

  onDestroy(() => { stopListWatch(); stopHealth(); });
  // Lifecycle: health runs whenever the Kubernetes view is open (cheap rail
  // badge); the list watcher runs only while the Flux sub-view is shown. A
  // one-shot snapshot probes CRD presence (drives the Flux tab) even before the
  // list is visible. Re-runs on active/visible/tab change.
  $effect(() => {
    const a = active, v = visible;
    if (!a) { stopListWatch(); stopHealth(); return; }
    startHealth();
    refreshList(tab);
    if (v) subscribeTab(tab);
    else stopListWatch();
  });
</script>

{#if present}
  <div class="flux">
    <div class="fx-tabs">
      {#each TABS as t (t.id)}
        <button class:on={tab === t.id} onclick={() => { tab = t.id; }}>{t.label}</button>
      {/each}
      {#if namespaces.length > 1}
        <select class="fx-ns-sel" bind:value={nsFilter} title="Filter by namespace">
          <option value="">all namespaces</option>
          {#each namespaces as ns (ns)}<option value={ns}>{ns}</option>{/each}
        </select>
      {/if}
      <span class="spacer"></span>
      {#if fails}<span class="fx-fail-chip" title="{fails} failing">{fails} failing</span>{/if}
      {#if loading}<span class="spin">…</span>{/if}
      {#if onRunCommand}<button class="fx-refresh" title="Watch all Flux events in terminal" onclick={() => onRunCommand?.(`flux events --watch${nsFilter ? ` -n ${nsFilter}` : ""}`)}><Icon name="alert" size={12} /></button>{/if}
      <button class="fx-refresh" title="Refresh" onclick={() => { refreshList(); refreshHealth(); }}><Icon name="refresh" size={12} /></button>
    </div>

    {#if err}<div class="fx-err">{err.slice(0, 200)}</div>{/if}

    <div class="fx-body">
        {#if loading && !items.length}
          <Skeleton rows={8} />
        {:else if !tabPresent}
          <EmptyState icon="flux" title="No {tab} CRDs in this cluster" hint="Flux's {tab} controller isn't installed here." />
        {:else if !shown.length}
          <EmptyState icon="flux" title="No {nsFilter ? `${tab} in ${nsFilter}` : tab} found" />
        {:else}
          {#each shown as it (it.ns + "/" + it.apiKind + "/" + it.name)}
            <div class="fx-row" class:busy={busyRow === it.ns + "/" + it.name}>
              <span class="fx-dot {it.suspended ? 'susp' : it.ready}" title={it.suspended ? "Suspended" : it.ready}></span>
              <span class="fx-name" title={it.message}>{it.name}</span>
              <span class="fx-ns">{it.ns}</span>
              {#if it.source && !readonly}<span class="fx-src" title="reconciles from source: {it.source}">← {it.source}</span>{/if}
              {#if it.deps}<span class="fx-deps" title={it.dependsOn?.length ? `depends on: ${it.dependsOn.join(", ")}` : `${it.deps} dependsOn`}>⇲ {it.dependsOn?.length ? it.dependsOn.join(", ") : it.deps}</span>{/if}
              {#if readonly}<span class="fx-k">{it.apiKind}</span>{/if}
              {#if it.ready === "fail" && it.message}
                <span class="fx-msg" title={it.message}>{oneLine(it.message)}</span>
              {:else}
                <span class="fx-rev mono" title={it.revision}>{shortRev(it.revision)}</span>
                <span class="spacer"></span>
              {/if}
              {#if it.apiKind === "HelmRelease" && onRunCommand}
                <button class="fx-act" title="Deployed values (helm get values)" onclick={() => onRunCommand?.(`helm get values ${it.name} -n ${it.ns}`)}>≡</button>
              {/if}
              {#if !readonly}
                <button class="fx-act" title="Reconcile (sync now)" disabled={!!busyRow} onclick={() => act(it, "flux_reconcile")}><Icon name="refresh" size={12} /></button>
                <button class="fx-act" title="Reconcile with source" disabled={!!busyRow} onclick={() => act(it, "flux_reconcile", true)}>↻+</button>
                {#if it.suspended}
                  <button class="fx-act" title="Resume" disabled={!!busyRow} onclick={() => act(it, "flux_resume")}><Icon name="play" size={12} /></button>
                {:else}
                  <button class="fx-act" title="Suspend" disabled={!!busyRow} onclick={() => act(it, "flux_suspend")}><Icon name="minus" size={12} /></button>
                {/if}
              {/if}
              {#if onInvestigate && it.ready === "fail"}
                <button class="fx-act ai" title="Investigate with the agent" onclick={() => onInvestigate(fluxInvestigation(it.apiKind, it.name, it.ns, it.message))}><Icon name="agent" size={11} /></button>
              {/if}
              <button class="fx-act" class:hot={it.ready === "fail"} title="Reconcile events (why) in terminal" onclick={() => events(it)}><Icon name="alert" size={11} /></button>
              <button class="fx-act" title="Stream logs in terminal" onclick={() => logs(it)}>↗</button>
            </div>
          {/each}
        {/if}
      </div>
  </div>
{/if}

<style>
  .flux { display: flex; flex-direction: column; flex: 1; min-height: 0; }
  .spacer { flex: 1; }
  .spin { color: var(--text3); }
  .fx-tabs { display: flex; align-items: center; gap: 2px; padding: 7px var(--pad-x, 10px); border-bottom: 1px solid var(--border); flex: 0 0 auto; }
  .fx-tabs button { background: transparent; border: 1px solid transparent; color: var(--text3);
    font-family: var(--font-ui); font-size: 11.5px; padding: 3px 10px; border-radius: 5px; cursor: default; }
  .fx-tabs button:hover { color: var(--text2); }
  .fx-tabs button.on { color: var(--text); background: var(--panel2); border-color: var(--border); }
  .fx-ns-sel { background: var(--panel2); color: var(--text2); border: 1px solid var(--border); border-radius: 5px; font-size: 11px; padding: 1px 4px; max-width: 150px; }
  .fx-refresh { color: var(--text3); display: inline-flex; align-items: center; padding: 3px; border: 0; background: transparent; border-radius: 4px; cursor: default; }
  .fx-refresh:hover { color: var(--text); background: color-mix(in srgb, var(--text) 8%, transparent); }
  .fx-err { margin: 6px var(--pad-x, 10px); color: var(--red); font-size: 11px; font-family: var(--font-mono); }
  .fx-body { flex: 1; min-height: 0; overflow-y: auto; }
  .fx-empty { padding: 10px var(--pad-x, 10px); color: var(--text3); font-size: 12px; }
  .fx-row { display: flex; align-items: center; gap: 8px; padding: 3px var(--pad-x, 10px); height: 24px; font-size: 12px; }
  .fx-row:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }
  .fx-row.busy { opacity: 0.5; }
  .fx-dot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; background: var(--text3); }
  .fx-dot.ok { background: var(--status-verified); }
  .fx-dot.fail { background: var(--status-failure); }
  .fx-dot.susp { background: var(--status-attention); }
  .fx-name { color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 200px; }
  .fx-ns { color: var(--text3); font-size: 11px; }
  .fx-k { color: var(--accent); font-size: 10px; font-family: var(--font-mono); }
  .fx-src { color: var(--text3); font-size: 10px; font-family: var(--font-mono); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 130px; }
  .fx-deps { color: var(--status-trace); font-size: 9.5px; font-family: var(--font-mono);
    max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 0 1 auto; }
  .fx-rev { color: var(--text2); font-size: 10.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 200px; }
  .fx-msg { flex: 1; min-width: 0; color: var(--status-failure); font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .fx-fail-chip { color: var(--red); font-size: 11px; font-family: var(--font-ui); padding: 1px 7px; border-radius: 9px;
    background: color-mix(in srgb, var(--red) 14%, transparent); border: 1px solid color-mix(in srgb, var(--red) 35%, transparent); }
  .mono { font-family: var(--font-mono); }
  .fx-act { background: transparent; border: 1px solid var(--border); color: var(--text2); border-radius: 5px;
    min-width: 22px; height: 20px; padding: 0 5px; font-size: 11px; display: inline-flex; align-items: center; justify-content: center; cursor: default; }
  .fx-act:hover:not(:disabled) { color: var(--text); border-color: var(--text3); }
  .fx-act:disabled { opacity: 0.4; }
  .fx-act.hot { color: var(--status-failure); border-color: color-mix(in srgb, var(--status-failure) 45%, transparent); }
  .fx-act.ai { color: var(--status-agent); border-color: color-mix(in srgb, var(--status-agent) 45%, transparent); }
</style>
