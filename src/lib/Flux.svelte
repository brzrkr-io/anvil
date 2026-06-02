<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import { toast } from "$lib/toast";
  import { askConfirm } from "$lib/dialog";
  import { byHealth, failingCount, oneLine, shortRev } from "$lib/flux-health";
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
  }

  let tab = $state<Tab>("kustomizations");
  let items = $state<FluxItem[]>([]);
  let loading = $state(false);
  let err = $state("");
  let present = $state(true); // false → cluster has no Flux CRDs; hide the panel
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

  function parse(raw: string): FluxItem[] {
    let j: any;
    try { j = JSON.parse(raw); } catch { return []; }
    const list = Array.isArray(j?.items) ? j.items : [];
    return list.map((it: any): FluxItem => {
      const conds = it?.status?.conditions ?? [];
      const ready = conds.find((c: any) => c.type === "Ready");
      const st = it?.status ?? {};
      const sp = it?.spec ?? {};
      const source = sp.sourceRef?.name || sp.chart?.spec?.sourceRef?.name || sp.chartRef?.name || "";
      return {
        name: it?.metadata?.name ?? "?",
        ns: it?.metadata?.namespace ?? "",
        apiKind: it?.kind ?? "",
        ready: !ready ? "unknown" : ready.status === "True" ? "ok" : "fail",
        suspended: it?.spec?.suspend === true,
        revision: st.lastAppliedRevision || st.lastAttemptedRevision || st.artifact?.revision || "",
        message: ready?.message ?? "",
        source,
        deps: Array.isArray(sp.dependsOn) ? sp.dependsOn.length : 0,
      };
    });
  }

  async function load() {
    loading = true;
    err = "";
    try {
      const raw = await invoke<string>("flux_get", { kind: tab });
      if (/the server doesn't have a resource type|no matches for kind|NotFound|could not find the requested resource/i.test(raw)) {
        present = false;
        items = [];
        return;
      }
      present = true;
      items = parse(raw).sort(byHealth);
    } catch (e) {
      err = String(e);
    } finally {
      loading = false;
      onPresence?.(present);
    }
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
      await load();
      refreshClusterHealth();
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

  // A1: cluster-wide failing count — Kustomizations + HelmReleases, regardless of
  // the active tab — so the rail badge reflects the whole cluster's health.
  let clusterFails = $state(0);
  async function refreshClusterHealth() {
    let total = 0;
    let any = false;
    for (const kind of ["kustomizations", "helmreleases"]) {
      try {
        const raw = await invoke<string>("flux_get", { kind });
        if (/the server doesn't have a resource type|no matches for kind|NotFound|could not find the requested resource/i.test(raw)) continue;
        any = true;
        total += failingCount(parse(raw));
      } catch { /* ignore one kind */ }
    }
    if (any) clusterFails = total;
  }
  $effect(() => { onHealth?.(present ? clusterFails : 0); });

  // Auto-poll so a reconcile is watched to green without hammering refresh — but
  // ONLY while the panel is actually on-screen. The Kubernetes view is kept-alive
  // (display:none) once opened, so without these gates Flux would fire 3 cluster-
  // wide kubectl calls every 6s for the whole session, even from a terminal.
  //   - list refresh: every POLL_MS, only when `visible` (Flux list shown)
  //   - cluster-health (rail badge): every HEALTH_EVERY ticks, only when `active`
  //     (Kubernetes view open) — much cheaper than the old 6s cadence.
  const POLL_MS = 6000;
  const HEALTH_EVERY = 3; // 3 × 6s = ~18s between health sweeps
  let healthTick = 0;
  let timer: ReturnType<typeof setInterval> | undefined;
  function tick() {
    if (!active || loading || busyRow || (typeof document !== "undefined" && document.hidden)) return;
    if (visible) load();
    if (++healthTick % HEALTH_EVERY === 0) refreshClusterHealth();
  }
  onMount(() => {
    timer = setInterval(tick, POLL_MS);
  });
  onDestroy(() => clearInterval(timer));
  // (Re)load when the panel becomes active or its list is shown. Reading both
  // `active` and `visible` makes this re-run on k8s open, on return to the view,
  // and when switching to the Flux sub-view (instant refresh instead of waiting a
  // poll tick). `load()` always runs while active so Flux-CRD presence is detected
  // even before the list is shown (drives onPresence → the Flux tab appears). It
  // doesn't read `loading`, so it can't loop on its own state changes.
  $effect(() => {
    const a = active; void visible;
    if (!a) return;
    load();
    refreshClusterHealth();
  });
</script>

{#if present}
  <div class="flux">
    <div class="fx-tabs">
      {#each TABS as t (t.id)}
        <button class:on={tab === t.id} onclick={() => { tab = t.id; load(); }}>{t.label}</button>
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
      <button class="fx-refresh" title="Refresh" onclick={load}><Icon name="refresh" size={12} /></button>
    </div>

    {#if err}<div class="fx-err">{err.slice(0, 200)}</div>{/if}

    <div class="fx-body">
        {#if loading && !items.length}
          <div class="fx-empty">Loading…</div>
        {:else if !shown.length}
          <div class="fx-empty">No {nsFilter ? `${tab} in ${nsFilter}` : tab} found.</div>
        {:else}
          {#each shown as it (it.ns + "/" + it.apiKind + "/" + it.name)}
            <div class="fx-row" class:busy={busyRow === it.ns + "/" + it.name}>
              <span class="fx-dot {it.suspended ? 'susp' : it.ready}" title={it.suspended ? "Suspended" : it.ready}></span>
              <span class="fx-name" title={it.message}>{it.name}</span>
              <span class="fx-ns">{it.ns}</span>
              {#if it.source && !readonly}<span class="fx-src" title="reconciles from source: {it.source}">← {it.source}</span>{/if}
              {#if it.deps}<span class="fx-deps" title="{it.deps} dependsOn">⇲{it.deps}</span>{/if}
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
  .fx-deps { color: var(--status-trace); font-size: 9.5px; font-family: var(--font-mono); }
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
