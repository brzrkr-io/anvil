<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { readCache, writeCache } from "$lib/cache";
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import { toast } from "$lib/toast";
  import { askConfirm, askText } from "$lib/dialog";
  import Flux from "$lib/Flux.svelte";
  import Helm from "$lib/Helm.svelte";
  import CodeView from "$lib/CodeView.svelte";
  import { applyRedaction } from "$lib/redaction";

  let { cwd, onRunCommand, onHealth, onInvestigate, onCheckConnections, active = true }: { cwd: string; onRunCommand?: (cmd: string) => void; onHealth?: (failing: number) => void; onInvestigate?: (prompt: string) => void; onCheckConnections?: () => void; active?: boolean } = $props();

  let contexts = $state<string[]>([]);
  let current = $state("");
  // A10: pin favorite kubeconfig contexts to the top of the switcher.
  const PIN_KEY = "anvil-kube-pinned";
  let pinned = $state<string[]>(
    (() => { try { return JSON.parse(localStorage.getItem(PIN_KEY) || "[]"); } catch { return []; } })(),
  );
  const sortedContexts = $derived(
    [...contexts].sort((a, b) => {
      const pa = pinned.includes(a), pb = pinned.includes(b);
      return pa === pb ? a.localeCompare(b) : pa ? -1 : 1;
    }),
  );
  function togglePin(name: string) {
    pinned = pinned.includes(name) ? pinned.filter((p) => p !== name) : [...pinned, name];
    localStorage.setItem(PIN_KEY, JSON.stringify(pinned));
  }
  let namespaces = $state<string[]>([]);
  let currentNs = $state("default");
  let busy = $state(false);
  let k8sErr = $state("");

  interface Pod {
    ns: string;
    name: string;
    ready: string;
    status: string;
    restarts: string;
    age: string;
  }
  interface PodPayload { rows: Pod[]; error: string }

  // Pods arrive from the backend watcher (watch.rs): parsed, sorted broken-first,
  // and capped at 400 in Rust off the UI thread, then pushed over `kube://pods`
  // only when they change. The frontend is a dumb subscriber — no polling, no
  // parse, no jank. Render straight from `livePods`.
  // Guard the cache: a pre-migration cache stored the raw pods TEXT, not Pod[].
  // Reject anything that isn't an array of rows with a name, so a stale value
  // can't feed undefined keys into the {#each} (each_key_duplicate crash).
  function cachedPods(): Pod[] {
    const c = readCache<unknown>("kube-pods");
    return Array.isArray(c) && c.every((p) => p && typeof (p as Pod).name === "string" && (p as Pod).name) ? (c as Pod[]) : [];
  }
  let livePods = $state<Pod[]>(cachedPods());
  const podRows = $derived(livePods);

  // Client-side filter only (cap already applied in Rust).
  let podFilter = $state("");
  const POD_CAP = 400;
  const filteredPods = $derived(
    podFilter.trim()
      ? podRows.filter((p) => `${p.name} ${p.ns}`.toLowerCase().includes(podFilter.toLowerCase()))
      : podRows,
  );
  const shownPods = $derived(filteredPods.slice(0, POD_CAP));

  const AUTH_RE = /expired|credentials|unauthorized|not logged in|sso session|reauthenticate|InvalidIdentityToken|token has expired|failed to get token/i;
  const authErr = $derived(AUTH_RE.test(k8sErr));

  const statusDot = (s: string): string =>
    s === "Running" || s === "Completed" ? "var(--green)"
      : /Error|CrashLoop|Failed|Evicted/.test(s) ? "var(--red)"
      : s === "Pending" || s === "ContainerCreating" ? "var(--yellow)"
      : "var(--text3)";

  const statusText = (s: string): string =>
    s === "Running" || s === "Completed" ? "var(--green)"
      : /Error|CrashLoop|Failed|Evicted/.test(s) ? "var(--red)"
      : s === "Pending" || s === "ContainerCreating" ? "var(--yellow)"
      : "var(--text3)";

  let panel = $state<{ pod: string; content: string; title: string } | null>(null);
  let pfList = $state<{ pid: string; desc: string }[]>([]);

  async function load() {
    busy = true;
    // Context/namespace metadata (cheap). Pods stream in from the watcher.
    const [ctxs, cur, curNs, nss] = await Promise.allSettled([
      invoke<string>("kube_contexts"),
      invoke<string>("kube_current_context"),
      invoke<string>("kube_current_namespace"),
      invoke<string>("kube_namespaces"),
    ]);
    contexts = ctxs.status === "fulfilled" ? ctxs.value.split("\n").filter(Boolean) : [];
    current = cur.status === "fulfilled" ? cur.value.trim() : "";
    currentNs = curNs.status === "fulfilled" ? curNs.value.trim() || "default" : "default";
    namespaces = nss.status === "fulfilled" ? nss.value.split("\n").filter(Boolean) : [];
    // No active context but kubeconfig has some → tell the user to pick one
    // instead of leaving the page blank with no explanation.
    if (!current && contexts.length) k8sErr = "Select a context above to load resources.";
    busy = false;
  }

  // One-shot shaped snapshot for instant pod refresh (Refresh button, context
  // switch). Falls back to the legacy text command if the backend predates the
  // watcher (e.g. dev backend not yet restarted) so the page always loads.
  async function refreshPods() {
    try {
      const p = await invoke<PodPayload>("kube_snapshot", { kind: "pods" });
      const rows = Array.isArray(p.rows) ? p.rows : [];
      k8sErr = p.error ?? "";
      // Keep last-known rows on a failed snapshot (see listen handler).
      if (k8sErr && rows.length === 0 && livePods.length) return;
      livePods = rows;
      writeCache("kube-pods", livePods);
    } catch {
      try {
        const text = await invoke<string>("kube_pods", { context: "" });
        if (AUTH_RE.test(text)) { livePods = []; k8sErr = "Cloud credentials expired or missing."; return; }
        livePods = parsePodsText(text); k8sErr = "";
        writeCache("kube-pods", livePods);
      } catch (e) { k8sErr = String(e); }
    }
  }

  // Client-side fallback parser (mirrors watch.rs): broken-first, then restarts.
  function parsePodsText(text: string): Pod[] {
    const lines = text.split("\n").filter(Boolean);
    if (!lines.length || !/^NAMESPACE\s/.test(lines[0])) return [];
    const rank = (s: string, r: string) =>
      /Error|CrashLoop|Failed|Evicted|ImagePull|Pending|Unknown|Init:|Terminating|OOMKilled/i.test(s) ? 0 : (parseInt(r, 10) || 0) > 0 ? 1 : 2;
    return lines.slice(1)
      .map((l) => l.split(/\s+/))
      .filter((c) => c[1])
      .map((c) => ({ ns: c[0], name: c[1], ready: c[2] ?? "", status: c[3] ?? "", restarts: c[4] ?? "0", age: c[c.length - 1] ?? "" }))
      .sort((a, b) => rank(a.status, a.restarts) - rank(b.status, b.restarts) || (parseInt(b.restarts, 10) || 0) - (parseInt(a.restarts, 10) || 0) || a.name.localeCompare(b.name))
      .slice(0, POD_CAP);
  }

  async function refreshPf() {
    try {
      pfList = (await invoke<string>("kube_pf_list")).split("\n").filter(Boolean).map((l) => {
        const [pid, desc] = l.split("\t");
        return { pid, desc };
      });
    } catch { pfList = []; }
  }

  async function useCtx(name: string) {
    if (!name || name === current) return;
    busy = true;
    try { await invoke("kube_use_context", { name }); current = name; await load(); refreshPods(); }
    catch (e) { k8sErr = String(e); }
    busy = false;
  }

  async function useNs(ns: string) {
    if (!ns || ns === currentNs) return;
    busy = true;
    try { await invoke("kube_set_namespace", { namespace: ns }); currentNs = ns; await load(); refreshPods(); }
    catch (e) { k8sErr = String(e); }
    busy = false;
  }

  async function openLogs(p: Pod) {
    panel = { pod: `${p.ns}/${p.name}`, content: "Loading…", title: "Logs" };
    try {
      const out = await invoke<string>("kube_logs", { context: current, namespace: p.ns, pod: p.name });
      panel = { pod: `${p.ns}/${p.name}`, content: applyRedaction(out), title: "Logs" };
    } catch (e) { panel = { pod: `${p.ns}/${p.name}`, content: String(e), title: "Logs" }; }
  }

  // #16 Cluster node capacity + usage in a panel.
  async function openNodes() {
    panel = { pod: current || "cluster", content: "Loading…", title: "Nodes" };
    try {
      const out = await invoke<string>("kube_nodes", { context: current });
      panel = { pod: current || "cluster", content: out, title: "Nodes" };
    } catch (e) { panel = { pod: current || "cluster", content: String(e), title: "Nodes" }; }
  }
  // #14 Rollout status: deployment READY/UP-TO-DATE/AVAILABLE columns.
  async function openRollouts() {
    panel = { pod: current || "cluster", content: "Loading…", title: "Rollouts" };
    try {
      const out = await invoke<string>("kube_deployments", { context: current });
      panel = { pod: current || "cluster", content: out, title: "Rollouts" };
    } catch (e) { panel = { pod: current || "cluster", content: String(e), title: "Rollouts" }; }
  }

  async function openEvents(p: Pod) {
    panel = { pod: `${p.ns}/${p.name}`, content: "Loading…", title: "Events" };
    try {
      const out = await invoke<string>("kube_events", { context: current, namespace: p.ns, object: p.name });
      panel = { pod: `${p.ns}/${p.name}`, content: out.trim() || "(no events)", title: "Events" };
    } catch (e) { panel = { pod: `${p.ns}/${p.name}`, content: String(e), title: "Events" }; }
  }

  async function openDescribe(p: Pod) {
    panel = { pod: `${p.ns}/${p.name}`, content: "Loading…", title: "Describe" };
    try {
      const out = await invoke<string>("kube_describe", { context: current, namespace: p.ns, pod: p.name });
      panel = { pod: `${p.ns}/${p.name}`, content: applyRedaction(out), title: "Describe" };
    } catch (e) { panel = { pod: `${p.ns}/${p.name}`, content: String(e), title: "Describe" }; }
  }

  async function restartPod(p: Pod) {
    const dep = p.name.replace(/-[a-z0-9]+-[a-z0-9]+$/, "");
    if (!await askConfirm({ title: "Rollout restart", message: `kubectl rollout restart deployment/${dep} in ${p.ns}?`, danger: true })) return;
    try {
      const out = await invoke<string>("kube_restart", { context: current, namespace: p.ns, deployment: dep });
      panel = { pod: `${p.ns}/${p.name}`, content: out, title: "Restart" };
      toast(`Restarted deployment/${dep}`, "success");
    } catch (e) { toast(String(e).slice(0, 100), "error"); }
  }

  async function deletePod(p: Pod) {
    if (!await askConfirm({ title: "Delete pod", message: `Delete pod ${p.name}? (its controller will recreate it)`, danger: true })) return;
    try {
      await invoke<string>("kube_delete_pod", { context: current, namespace: p.ns, pod: p.name });
      toast(`Deleted ${p.name}`, "success");
      await load();
    } catch (e) { toast(String(e).slice(0, 100), "error"); }
  }

  async function portForward(p: Pod) {
    const ports = await askText({ title: "Port-forward", message: "local:remote, e.g. 8080:80", placeholder: "8080:80" });
    if (!ports || !/^\d+:\d+$/.test(ports.trim())) return;
    try {
      await invoke("kube_pf_start", { context: current, namespace: p.ns, pod: p.name, ports: ports.trim() });
      await refreshPf();
      toast(`Port-forwarding ${p.name} → ${ports.trim()}`, "success");
    } catch (e) { toast(String(e).slice(0, 100), "error"); }
  }

  async function stopPf(pid: string) {
    try { await invoke("kube_pf_stop", { pid: Number(pid) }); } catch (e) { console.warn("kube_pf_stop failed", e); }
    await refreshPf();
  }

  function execPod(p: Pod) {
    onRunCommand?.(`kubectl exec -it -n ${p.ns} ${p.name} -- sh -c 'command -v bash >/dev/null && exec bash || exec sh'`);
  }
  // A7: attach an ephemeral debug container (netshoot) — for distroless/crashing
  // pods where exec into the app container won't work.
  function debugPod(p: Pod) {
    const ctx = current ? `--context ${current} ` : "";
    onRunCommand?.(`kubectl ${ctx}debug -it -n ${p.ns} ${p.name} --image=nicolaka/netshoot --share-processes -- bash`);
  }

  // Stream logs live in a terminal pane (the in-panel Logs view is a snapshot).
  function followLogs(p: Pod) {
    const ctx = current ? `--context ${current} ` : "";
    onRunCommand?.(`kubectl ${ctx}logs -f --tail=200 -n ${p.ns} ${p.name}`);
  }

  // Flux | Workloads top-level views. Default to Workloads; switch to Flux the
  // first time the cluster reports Flux CRDs (GitOps-first), but never trap the
  // user there if Flux disappears.
  let view = $state<"flux" | "workloads" | "helm">("workloads");
  let fluxPresent = $state(false);
  let fluxDefaulted = false;
  function onFluxPresence(p: boolean) {
    fluxPresent = p;
    if (p && !fluxDefaulted) { fluxDefaulted = true; view = "flux"; }
    else if (!p && view === "flux") view = "workloads";
  }

  // Data layer: the backend pushes shaped pod rows over `kube://pods`. We start
  // the watcher only while the Kubernetes view is on-screen and stop it when it
  // isn't, so nothing runs in the background. Last-known rows render instantly
  // from cache on mount.
  let unlistenPods: (() => void) | undefined;
  let watching = false;
  function startPodWatch() {
    if (watching) return;
    watching = true;
    invoke("kube_watch_start", { kind: "pods", intervalMs: 5000 }).catch(() => {});
  }
  function stopPodWatch() {
    if (!watching) return;
    watching = false;
    invoke("kube_watch_stop", { kind: "pods" }).catch(() => {});
  }
  onMount(async () => {
    load();
    refreshPf();
    try {
      unlistenPods = await listen<PodPayload>("kube://pods", (e) => {
        const rows = Array.isArray(e.payload?.rows) ? e.payload.rows : [];
        const err = e.payload?.error ?? "";
        k8sErr = err;
        // Keep the last good rows on a failed poll (expired creds, transient
        // network) so the page stays populated (dimmed) instead of blanking
        // out mid-session. Only replace when we actually got data, or when the
        // cluster legitimately has no pods (no error).
        if (err && rows.length === 0 && livePods.length) return;
        livePods = rows;
        writeCache("kube-pods", livePods);
      });
    } catch { /* no Tauri event bus (e.g. browser preview) — snapshot still works */ }
  });
  onDestroy(() => { stopPodWatch(); unlistenPods?.(); });
  // Start/stop the watcher with view visibility; refresh instantly on (re)entry.
  $effect(() => {
    if (active) { refreshPods(); startPodWatch(); }
    else stopPodWatch();
  });
</script>

<div class="kube">
  <!-- Top bar: context, namespace, refresh -->
  <div class="topbar">
    <span class="lbl">Context</span>
    <select value={current} onchange={(e) => useCtx((e.currentTarget as HTMLSelectElement).value)} disabled={busy}>
      {#each sortedContexts as c (c)}<option value={c}>{pinned.includes(c) ? "★ " : ""}{c}</option>{/each}
      {#if !contexts.length && current}<option value={current}>{current}</option>{/if}
    </select>
    {#if current}
      <button class="pin" class:on={pinned.includes(current)} title={pinned.includes(current) ? "Unpin context" : "Pin context to top"} onclick={() => togglePin(current)}>★</button>
    {/if}
    <span class="lbl">Namespace</span>
    <select value={currentNs} onchange={(e) => useNs((e.currentTarget as HTMLSelectElement).value)} disabled={busy}>
      {#if !namespaces.includes(currentNs)}<option value={currentNs}>{currentNs}</option>{/if}
      {#each namespaces as n (n)}<option value={n}>{n}</option>{/each}
    </select>
    <span class="spacer"></span>
    {#if busy}<span class="spin">…</span>{/if}
    <button class="iconbtn" onclick={openRollouts} title="Rollout status (deployments READY/UP-TO-DATE)">
      <Icon name="workspace" size={13} />
    </button>
    <button class="iconbtn" onclick={openNodes} title="Node capacity & usage (kubectl top nodes)">
      <Icon name="chart" size={13} />
    </button>
    <button class="iconbtn" onclick={() => { load(); refreshPods(); refreshPf(); }} title="Refresh" disabled={busy}>
      <Icon name="refresh" size={13} />
    </button>
  </div>

  <!-- Auth error bar -->
  {#if authErr}
    <div class="authbar">
      <Icon name="alert" size={13} />
      <span>Cloud credentials expired or missing.</span>
      <button onclick={() => onRunCommand?.("aws sso login")}>aws sso login</button>
      <button onclick={() => onRunCommand?.(`aws eks update-kubeconfig --name "${current}"`)}>refresh kubeconfig</button>
      <button class="ghost" onclick={() => { load(); refreshPods(); }}>Retry</button>
      {#if onCheckConnections}<button class="ghost" onclick={onCheckConnections}>Check connections</button>{/if}
      {#if livePods.length}<span class="stale">showing last-known pods</span>{/if}
    </div>
  {/if}

  <!-- Flux | Workloads | Helm view switch (Flux tab only when the cluster runs it). -->
  <div class="kviews">
    {#if fluxPresent}
      <button class:on={view === "flux"} onclick={() => (view = "flux")}><Icon name="flux" size={12} /> Flux</button>
    {/if}
    <button class:on={view === "workloads"} onclick={() => (view = "workloads")}><Icon name="workspace" size={12} /> Workloads</button>
    <button class:on={view === "helm"} onclick={() => (view = "helm")}><Icon name="helm" size={12} /> Helm</button>
  </div>

  <!-- FluxCD (GitOps) view. Always mounted (to detect Flux CRDs), shown only in
       the Flux view. Self-hides if the cluster has no Flux. -->
  <div class="kpane" style:display={view === "flux" && fluxPresent ? "flex" : "none"}>
    <Flux {onRunCommand} onPresence={onFluxPresence} {onHealth} {onInvestigate} {active} visible={active && view === "flux"} />
  </div>

  <!-- Helm releases view. -->
  <div class="kpane" style:display={view === "helm" ? "flex" : "none"}>
    <Helm />
  </div>

  <!-- Workloads view: port-forwards + pods. -->
  <div class="kpane" style:display={view === "workloads" ? "flex" : "none"}>
  {#if pfList.length}
    <div class="section-head">
      <span class="sect-lbl">Port-forwards</span>
      <span class="sect-cnt">{pfList.length}</span>
    </div>
    <div class="pflist">
      {#each pfList as f (f.pid)}
        <div class="pfrow">
          <span class="pf-dot"></span>
          <span class="pf-desc">{f.desc || f.pid}</span>
          <button class="iconbtn danger" onclick={() => stopPf(f.pid)} title="Stop"><Icon name="close" size={12} /></button>
        </div>
      {/each}
    </div>
  {/if}

  <!-- Main pod table / panel split -->
  <div class="body">
    <!-- Pod table -->
    <div class="pods" class:split={!!panel} class:stale={authErr && livePods.length}>
      {#if k8sErr && !authErr}
        <div class="empty">{k8sErr}</div>
      {:else if podRows.length}
        <div class="pod-filter">
          <input class="pf-in" placeholder="Filter pods… ({podRows.length})" bind:value={podFilter} spellcheck="false" />
          {#if filteredPods.length > POD_CAP}<span class="pf-cap">showing {POD_CAP} of {filteredPods.length} — filter to narrow</span>{/if}
        </div>
        <!-- Table header -->
        <div class="pod-header">
          <span class="col-dot"></span>
          <span class="col-name">Name</span>
          <span class="col-ready">Ready</span>
          <span class="col-restarts">Restarts</span>
          <span class="col-status">Status</span>
          <span class="col-age">Age</span>
          <span class="col-acts"></span>
        </div>
        {#each shownPods as p, i (`${p.ns}/${p.name}/${i}`)}
          <div class="pod-row" role="button" tabindex="0"
            onclick={() => openLogs(p)}
            onkeydown={(e) => e.key === "Enter" && openLogs(p)}
            title="View logs"
          >
            <span class="col-dot">
              <span class="dot" style="background:{statusDot(p.status)}"></span>
            </span>
            <span class="col-name mono">{p.name}</span>
            <span class="col-ready muted">{p.ready}</span>
            <span class="col-restarts" style="color:{Number(p.restarts) > 0 ? 'var(--red)' : 'var(--text3)'}">
              {p.restarts}
            </span>
            <span class="col-status" style="color:{statusText(p.status)}">{p.status}</span>
            <span class="col-age muted">{p.age}</span>
            <span class="col-acts">
              <button class="act" title="Logs (snapshot)" onclick={(e) => { e.stopPropagation(); openLogs(p); }}>
                <Icon name="history" size={12} />
              </button>
              <button class="act" title="Follow logs in terminal" onclick={(e) => { e.stopPropagation(); followLogs(p); }}>
                <Icon name="play" size={12} />
              </button>
              <button class="act" title="Describe" onclick={(e) => { e.stopPropagation(); openDescribe(p); }}>
                <Icon name="info" size={12} />
              </button>
              <button class="act" title="Events (why)" onclick={(e) => { e.stopPropagation(); openEvents(p); }}>
                <Icon name="alert" size={12} />
              </button>
              <button class="act" title="Exec shell" onclick={(e) => { e.stopPropagation(); execPod(p); }}>
                <Icon name="terminal" size={12} />
              </button>
              <button class="act" title="Debug (ephemeral netshoot container)" onclick={(e) => { e.stopPropagation(); debugPod(p); }}>
                <Icon name="agent" size={12} />
              </button>
              <button class="act" title="Port-forward" onclick={(e) => { e.stopPropagation(); portForward(p); }}>
                <Icon name="branch" size={12} />
              </button>
              <button class="act warn" title="Rollout restart deployment" onclick={(e) => { e.stopPropagation(); restartPod(p); }}>
                <Icon name="refresh" size={12} />
              </button>
              <button class="act danger" title="Delete pod" onclick={(e) => { e.stopPropagation(); deletePod(p); }}>
                <Icon name="close" size={12} />
              </button>
            </span>
          </div>
        {/each}
      {:else if !busy}
        <div class="empty">
          {k8sErr ? k8sErr : "No pods found."}
        </div>
      {:else}
        <div class="empty">Loading…</div>
      {/if}
    </div>

    <!-- Inline log/describe panel -->
    {#if panel}
      <div class="log-panel">
        <div class="log-head">
          <span class="log-title">{panel.title}</span>
          <span class="log-pod">{panel.pod}</span>
          <span class="spacer"></span>
          <button class="iconbtn" onclick={() => (panel = null)} title="Close"><Icon name="close" size={13} /></button>
        </div>
        {#if panel.title === "Describe"}
          <div class="log-out cv"><CodeView text={panel.content} lang="yaml" /></div>
        {:else}
          <pre class="log-out">{panel.content}</pre>
        {/if}
      </div>
    {/if}
  </div>
  </div>
</div>

<style>
  .kube { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .kviews { display: flex; gap: 4px; padding: 6px 12px; border-bottom: 1px solid var(--border); flex: 0 0 auto; }
  .kviews button { display: inline-flex; align-items: center; gap: 5px; background: transparent;
    border: 1px solid transparent; color: var(--text3); font-family: var(--font-ui); font-size: 12px;
    padding: 3px 11px; border-radius: 6px; cursor: default; }
  .kviews button:hover { color: var(--text2); }
  .kviews button.on { color: var(--text); background: var(--panel2); border-color: var(--border); }
  .kpane { flex: 1; min-height: 0; flex-direction: column; }

  /* Top bar */
  .topbar {
    display: flex; align-items: center; gap: 8px; height: 28px; flex: 0 0 auto;
    padding: 0 12px; border-bottom: 1px solid var(--border);
  }
  .lbl { color: var(--text3); font-size: 11px; font-weight: 500; flex: 0 0 auto; }
  .topbar select {
    background: var(--panel2); color: var(--accent); border: 1px solid var(--border);
    border-radius: var(--radius, 6px); padding: 2px 6px; font-size: 11.5px;
    font-family: var(--font-mono); outline: 0; max-width: 220px;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .topbar select:disabled { opacity: 0.6; }
  .pin {
    background: none; border: none; cursor: pointer; padding: 0 2px;
    color: var(--text3); font-size: 13px; line-height: 1; flex: 0 0 auto;
  }
  .pin.on { color: var(--status-attention, var(--yellow, #d8a657)); }
  .spacer { flex: 1; }
  .spin { color: var(--accent); font-size: 12px; }

  /* Icon buttons */
  .iconbtn {
    display: inline-flex; align-items: center; justify-content: center;
    width: 22px; height: 20px; border: 0; border-radius: 5px;
    background: transparent; color: var(--text3); cursor: default;
  }
  .iconbtn:hover:not(:disabled) { background: var(--sel); color: var(--text); }
  .iconbtn:disabled { opacity: 0.4; }
  .iconbtn.danger:hover { color: var(--red); }

  /* Auth error bar */
  .authbar {
    display: flex; align-items: center; gap: 8px; padding: 6px 12px;
    background: color-mix(in srgb, var(--red) 12%, var(--bg));
    border-bottom: 1px solid var(--border); font-size: 11.5px; color: var(--red);
    flex: 0 0 auto;
  }
  .authbar span { margin-right: 4px; }
  .authbar button {
    border: 1px solid var(--accent); background: var(--accent); color: var(--bg);
    font-family: var(--font-mono); font-size: 11px; padding: 2px 8px;
    border-radius: var(--radius, 6px); cursor: default;
  }
  .authbar button.ghost { background: transparent; color: var(--text2); border-color: var(--border); }
  .authbar button:hover { filter: brightness(1.08); }
  .authbar .stale { color: var(--text3); font-style: italic; margin-left: auto; }
  /* Stale data (creds expired) — dim the last-known rows until they refresh. */
  .pods.stale { opacity: 0.5; transition: opacity 0.2s; }

  /* Port-forwards */
  .section-head {
    display: flex; align-items: center; gap: 5px; padding: 4px 12px 2px;
    font-size: 11px; color: var(--text3); font-weight: 500;
    border-bottom: 1px solid var(--hairline); flex: 0 0 auto;
  }
  .sect-cnt { color: var(--text3); font-size: 10px; }
  .pflist { flex: 0 0 auto; padding: 2px 0 4px; border-bottom: 1px solid var(--border); }
  .pfrow {
    display: flex; align-items: center; gap: 8px; padding: 3px 12px;
    font-size: 11.5px; font-family: var(--font-mono);
  }
  .pfrow:hover { background: color-mix(in srgb, var(--text) 6%, transparent); }
  .pf-dot {
    flex: 0 0 auto; width: 6px; height: 6px; border-radius: 50%;
    background: var(--accent);
  }
  .pf-desc { flex: 1; min-width: 0; color: var(--text2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* Body: pod table + log panel side-by-side when panel open */
  .body { flex: 1; min-height: 0; display: flex; overflow: hidden; }

  /* Pod table */
  .pods { flex: 1; min-width: 0; overflow-y: auto; }
  .pods.split { flex: 0 0 55%; border-right: 1px solid var(--border); }

  .pod-filter {
    display: flex; align-items: center; gap: 10px; padding: 6px 12px;
    border-bottom: 1px solid var(--hairline); position: sticky; top: 0; z-index: 2; background: var(--panel);
  }
  .pf-in {
    flex: 1; height: 24px; border: 1px solid var(--border); background: var(--panel2);
    color: var(--text); border-radius: 5px; padding: 0 8px; font-size: 11.5px; font-family: var(--font-mono);
  }
  .pf-in:focus { outline: none; border-color: var(--text3); }
  .pf-cap { flex: 0 0 auto; font-size: 10px; color: var(--text3); }

  /* Shared grid so header + every row align exactly. Actions float on hover
     (absolute) so they never shift the columns. */
  .pod-header, .pod-row {
    display: grid; grid-template-columns: 18px minmax(0, 1fr) 48px 64px 120px 52px;
    align-items: center; column-gap: 10px; height: 22px; padding: 0 12px;
    border-bottom: 1px solid var(--hairline); position: relative;
  }
  .pod-header {
    font-size: 10.5px; color: var(--text3); font-weight: 500;
    position: sticky; top: 0; background: var(--panel); z-index: 1;
  }
  .pod-row { font-size: 11.5px; cursor: default; transition: background 0.1s ease; }
  .pod-row:hover { background: color-mix(in srgb, var(--text) 6%, transparent); }
  .pod-row:hover .col-acts { opacity: 1; }

  /* Columns */
  .col-dot { display: flex; align-items: center; }
  .dot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; }
  .col-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
  .col-ready { text-align: right; }
  .col-restarts { text-align: right; font-family: var(--font-mono); }
  .col-status { text-align: right; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .col-age { text-align: right; }
  .col-acts { position: absolute; right: 8px; top: 0; height: 100%; display: flex; align-items: center; gap: 2px;
    padding-left: 14px; background: linear-gradient(to right, transparent, var(--panel) 16px);
    opacity: 0; transition: opacity 0.1s; }
  .pod-row:hover .col-acts { background: linear-gradient(to right, transparent, color-mix(in srgb, var(--text) 6%, var(--panel)) 16px); }

  .mono { font-family: var(--font-mono); }
  .muted { color: var(--text3); }

  /* Row action buttons (revealed on hover) */
  .act {
    display: inline-flex; align-items: center; justify-content: center;
    width: 20px; height: 18px; border: 1px solid var(--border);
    background: var(--panel2); color: var(--text2); border-radius: 4px; cursor: default;
  }
  .act:hover { color: var(--text); border-color: var(--text3); }
  .act.warn:hover { color: var(--yellow); border-color: var(--yellow); }
  .act.danger:hover { color: var(--red); border-color: var(--red); }

  /* Empty / error */
  .empty { padding: 24px 16px; color: var(--text3); font-size: 12px; }

  /* Inline log/describe panel */
  .log-panel { flex: 1; min-width: 0; display: flex; flex-direction: column; min-height: 0; }
  .log-head {
    display: flex; align-items: center; gap: 8px; height: 28px; flex: 0 0 auto;
    padding: 0 12px; border-bottom: 1px solid var(--border); font-size: 11.5px;
  }
  .log-title { color: var(--text3); font-size: 11px; font-weight: 500; flex: 0 0 auto; }
  .log-pod { color: var(--text); font-family: var(--font-mono); font-size: 11px; flex: 0 0 auto; }
  .log-out {
    flex: 1; min-height: 0; overflow: auto; margin: 0; padding: 10px 12px;
    font-family: var(--font-mono); font-size: 11px; line-height: 1.45;
    color: var(--text2); white-space: pre; background: var(--bg);
  }
  /* CodeView owns its scroller/padding/font — strip the <pre> styling. */
  .log-out.cv { padding: 0; overflow: hidden; white-space: normal; }
</style>
