<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { classifyK8sError, friendlyK8sError, parseNamespaces } from "$lib/k8s-errors";
  import { reauthActions, type CloudAuth } from "$lib/kube-cloud";
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import Skeleton from "$lib/Skeleton.svelte";
  import EmptyState from "$lib/EmptyState.svelte";
  import Resizer from "$lib/Resizer.svelte";
  import { toast } from "$lib/toast";
  import { askConfirm, askText } from "$lib/dialog";
  import Flux from "$lib/Flux.svelte";
  import Helm from "$lib/Helm.svelte";
  import CodeView from "$lib/CodeView.svelte";
  import { applyRedaction } from "$lib/redaction";

  let { cwd, onRunCommand, onHealth, onInvestigate, onCheckConnections, active = true }: { cwd: string; onRunCommand?: (cmd: string) => void; onHealth?: (failing: number) => void; onInvestigate?: (prompt: string) => void; onCheckConnections?: () => void; active?: boolean } = $props();

  let contexts = $state<string[]>([]);
  // `current` = the context Anvil VIEWS (queries via --context); `shellCtx` =
  // the kubeconfig's own current-context (what the user's terminals use). In the
  // hybrid view-only model these can differ: picking a cluster here never writes
  // kubeconfig, so the shell stays put until you explicitly sync it.
  let current = $state("");
  let shellCtx = $state("");
  // A10: pin favorite kubeconfig contexts to the top of the switcher.
  const PIN_KEY = "anvil-kube-pinned";
  let pinned = $state<string[]>(
    (() => { try { return JSON.parse(localStorage.getItem(PIN_KEY) || "[]"); } catch { return []; } })(),
  );
  // Recently-viewed contexts (most-recent-first) float just under the pinned
  // ones so the clusters you actually use sit at the top of the switcher.
  const RECENT_KEY = "anvil-kube-recent";
  let recent = $state<string[]>(
    (() => { try { return JSON.parse(localStorage.getItem(RECENT_KEY) || "[]"); } catch { return []; } })(),
  );
  function rememberRecent(name: string) {
    recent = [name, ...recent.filter((r) => r !== name)].slice(0, 6);
    try { localStorage.setItem(RECENT_KEY, JSON.stringify(recent)); } catch { /* ignore */ }
  }
  const sortedContexts = $derived(
    [...contexts].sort((a, b) => {
      const pa = pinned.includes(a), pb = pinned.includes(b);
      if (pa !== pb) return pa ? -1 : 1;
      const ra = recent.indexOf(a), rb = recent.indexOf(b);
      const fa = ra === -1 ? 99 : ra, fb = rb === -1 ? 99 : rb;
      return fa !== fb ? fa - fb : a.localeCompare(b);
    }),
  );
  function togglePin(name: string) {
    pinned = pinned.includes(name) ? pinned.filter((p) => p !== name) : [...pinned, name];
    localStorage.setItem(PIN_KEY, JSON.stringify(pinned));
  }
  let namespaces = $state<string[]>([]);
  // "" = All namespaces (pods are listed cluster-wide; this is a client-side
  // filter + the default ns for per-pod actions). Persisted, view-only.
  let currentNs = $state((() => { try { return localStorage.getItem("anvil-kube-ns") ?? ""; } catch { return ""; } })());
  // Resizable width of the pods table when the log/describe panel is open.
  let podsW = $state((() => { try { return Number(localStorage.getItem("anvil-kube-podsw")) || 640; } catch { return 640; } })());
  // #24 Pin favorite namespaces to the top of the switcher (mirrors contexts).
  const PIN_NS_KEY = "anvil-kube-pinned-ns";
  let pinnedNs = $state<string[]>(
    (() => { try { return JSON.parse(localStorage.getItem(PIN_NS_KEY) || "[]"); } catch { return []; } })(),
  );
  const sortedNamespaces = $derived(
    [...namespaces].sort((a, b) => {
      const pa = pinnedNs.includes(a), pb = pinnedNs.includes(b);
      return pa === pb ? a.localeCompare(b) : pa ? -1 : 1;
    }),
  );
  function togglePinNs(name: string) {
    pinnedNs = pinnedNs.includes(name) ? pinnedNs.filter((p) => p !== name) : [...pinnedNs, name];
    localStorage.setItem(PIN_NS_KEY, JSON.stringify(pinnedNs));
  }
  let busy = $state(false);
  let k8sErr = $state("");
  let cloudInfo = $state<CloudAuth | null>(null); // per-context AWS/cloud auth detail

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
  // Auth-gated: NEVER seed from cache or render stale cluster data. Pods start
  // empty and appear only on a live, authenticated read of the CURRENT cluster
  // this session (conn === "live"). Showing a dead session's pods — or the
  // previous context's — then flashing to a login prompt is exactly wrong.
  let livePods = $state<Pod[]>([]);
  const podRows = $derived(livePods);

  // Per-cluster connection state — the single source of truth for what renders.
  // Data shows ONLY when "live". Switching context resets to "connecting" and
  // clears pods, so the old cluster never lingers on screen.
  type ConnState = "connecting" | "live" | "auth" | "error";
  let conn = $state<ConnState>("connecting");
  function applyPods(rows: Pod[], err: string) {
    if (err) {
      k8sErr = err;
      livePods = []; // never show stale data behind an error
      conn = classifyK8sError(err) === "auth" ? "auth" : "error";
      return;
    }
    k8sErr = "";
    livePods = rows;
    conn = "live";
  }

  // Client-side filter only (cap already applied in Rust).
  let podFilter = $state("");
  const POD_CAP = 400;
  const filteredPods = $derived(
    podRows.filter(
      (p) =>
        (!currentNs || p.ns === currentNs) &&
        (!podFilter.trim() || `${p.name} ${p.ns}`.toLowerCase().includes(podFilter.toLowerCase())),
    ),
  );
  // Stable, de-duplicated rows. The {#each} keys on ns/name ALONE (no index) so a
  // pod that re-ranks (broken-first) is MOVED, not destroyed+recreated — that DOM
  // churn was the "jumpy" flashing + scroll reset every 5s. Dedupe guards the
  // keyed each against a malformed parse yielding two rows with the same ns/name.
  const shownPods = $derived.by(() => {
    const seen = new Set<string>();
    const out: Pod[] = [];
    for (const p of filteredPods) {
      const k = `${p.ns}/${p.name}`;
      if (seen.has(k)) continue;
      seen.add(k);
      out.push(p);
      if (out.length >= POD_CAP) break;
    }
    return out;
  });

  // Auth-error detection drives the re-auth banner; full classification (auth /
  // rbac / network) lives in the shared, unit-tested k8s-errors helper (#5).
  const authErr = $derived(conn === "auth");
  const friendlyErr = friendlyK8sError;

  // One semantic status classifier drives the dot, the chip, and the summary —
  // brand status colors (verified / attention / failure).
  function statusKind(s: string): "ok" | "warn" | "bad" | "idle" {
    if (s === "Running") return "ok";
    if (/Error|CrashLoop|Failed|Evicted|OOMKilled|ImagePull|BackOff|Unknown/i.test(s)) return "bad";
    if (/Pending|ContainerCreating|Init|Terminating|PodInitializing/i.test(s)) return "warn";
    return "idle"; // Completed / Succeeded / other steady states
  }
  // Operational glance over the whole cluster (pre-filter): the health summary.
  const podStats = $derived.by(() => {
    let running = 0, pending = 0, failing = 0;
    for (const p of podRows) {
      const k = statusKind(p.status);
      if (k === "bad") failing++;
      else if (k === "warn") pending++;
      else running++;
    }
    return { running, pending, failing, total: podRows.length };
  });

  let panel = $state<{ pod: string; content: string; title: string } | null>(null);
  let pfList = $state<{ pid: string; desc: string }[]>([]);

  async function load() {
    busy = true;
    // Context/namespace metadata (cheap). Pods stream in from the watcher.
    // kube_namespaces is context-aware (the backend injects the view context),
    // so it lists the namespaces of the cluster Anvil is VIEWING, not the shell.
    const [ctxs, cur, nss] = await Promise.allSettled([
      invoke<string>("kube_contexts"),
      invoke<string>("kube_current_context"),
      invoke<string>("kube_namespaces"),
    ]);
    contexts = ctxs.status === "fulfilled" ? ctxs.value.split("\n").filter(Boolean) : [];
    // Ambient kubeconfig context = what the user's terminals use.
    shellCtx = cur.status === "fulfilled" ? cur.value.trim() : "";
    // Seed the viewed context from the shell on first load so the page just
    // works; once the user has picked a cluster, keep their selection.
    if (!current) current = shellCtx;
    // parseNamespaces keeps only valid label names: on an auth failure the
    // backend returns kubectl error TEXT, which must not become namespace options.
    namespaces = nss.status === "fulfilled" ? parseNamespaces(nss.value) : [];
    // Per-context AWS/cloud auth detail (profile, region, cluster, sso_session,
    // live auth status) for the CURRENT context only — drives precise SSO login.
    if (current) {
      invoke<CloudAuth>("kube_context_cloud", { context: current })
        .then((ci) => (cloudInfo = ci))
        .catch(() => (cloudInfo = null));
    } else cloudInfo = null;
    // No context to view at all (empty kubeconfig / none restored) → prompt.
    if (!current && contexts.length) {
      k8sErr = "Select a context above to load resources.";
    }
    busy = false;
  }

  // One-shot shaped snapshot for instant pod refresh (Refresh button, context
  // switch). Falls back to the legacy text command if the backend predates the
  // watcher (e.g. dev backend not yet restarted) so the page always loads.
  async function refreshPods() {
    try {
      const p = await invoke<PodPayload>("kube_snapshot", { kind: "pods" });
      applyPods(Array.isArray(p.rows) ? p.rows : [], p.error ?? "");
    } catch {
      // Backend predates the watcher (dev backend not restarted) — legacy text
      // path, still auth-gated (no stale, no cache).
      try {
        const text = await invoke<string>("kube_pods", { context: "" });
        if (classifyK8sError(text) === "auth") applyPods([], "Cloud credentials expired or missing.");
        else applyPods(parsePodsText(text), "");
      } catch (e) { applyPods([], String(e)); }
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
    // View-only: tell the backend which cluster to QUERY (it injects --context
    // on every kubectl call) — never `kubectl config use-context`, so the user's
    // terminals stay on whatever context they were already using.
    try {
      await invoke("kube_set_view_context", { name });
      current = name;
      rememberRecent(name);
      try { localStorage.setItem("anvil-kube-context", name); } catch { /* ignore */ }
      currentNs = ""; // namespaces differ per cluster — reset to All
      panel = null;
      conn = "connecting"; livePods = []; // drop the old cluster instantly — no stale bleed
      await load();
      refreshPods();
    } catch (e) { k8sErr = String(e); }
    busy = false;
  }

  function useNs(ns: string) {
    // View-only: the namespace is a client-side filter + default for per-pod
    // actions; pods are listed cluster-wide (-A). Don't pin it into kubeconfig.
    currentNs = ns;
    try { localStorage.setItem("anvil-kube-ns", ns); } catch { /* ignore */ }
  }

  // Opt-in escape hatch from the view-only model: actually move the shell's
  // kubeconfig current-context to match what Anvil is viewing, so the user's
  // terminals follow. The ONLY place Anvil writes kubeconfig.
  async function syncShellContext() {
    if (!current) return;
    try {
      await invoke("kube_use_context", { name: current });
      shellCtx = current;
      toast(`Shell context → ${current}`, "success");
    } catch (e) { toast(String(e).slice(0, 100), "error"); }
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
  // Bumped by Refresh/Retry to force the Flux child to re-fetch immediately
  // (e.g. recover right after the user re-auths in a terminal).
  let fluxNonce = $state(0);
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
    // Restore the last cluster Anvil was VIEWING (view-only — does not move the
    // shell). load() then lists that cluster's namespaces + streams its pods.
    const remembered = (() => { try { return localStorage.getItem("anvil-kube-context") || ""; } catch { return ""; } })();
    if (remembered) { current = remembered; invoke("kube_set_view_context", { name: remembered }).catch(() => {}); }
    load();
    refreshPf();
    try {
      unlistenPods = await listen<PodPayload>("kube://pods", (e) => {
        // Auth-gated: an error (expired creds, unreachable) clears the pods and
        // flips to the auth/error state — we never keep showing the last batch.
        applyPods(Array.isArray(e.payload?.rows) ? e.payload.rows : [], e.payload?.error ?? "");
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
    <select value={current} onchange={(e) => useCtx((e.currentTarget as HTMLSelectElement).value)} disabled={busy} title="Cluster Anvil is viewing — does not change your shell's kubectl context">
      {#each sortedContexts as c, i (c + '#' + i)}<option value={c}>{pinned.includes(c) ? "★ " : ""}{c}</option>{/each}
      {#if current && !contexts.includes(current)}<option value={current}>{current}</option>{/if}
    </select>
    {#if current}
      <button class="pin" class:on={pinned.includes(current)} title={pinned.includes(current) ? "Unpin context" : "Pin context to top"} onclick={() => togglePin(current)}>★</button>
    {/if}
    {#if current && shellCtx && current !== shellCtx}
      <button class="shellsync" onclick={syncShellContext}
        title={`Your shell/kubectl is on "${shellCtx}". Anvil is only viewing "${current}". Click to point your shell here too (kubectl config use-context).`}>
        <Icon name="terminal" size={11} /> shell: {shellCtx}
      </button>
    {/if}
    <span class="lbl">Namespace</span>
    <select value={currentNs} onchange={(e) => useNs((e.currentTarget as HTMLSelectElement).value)} disabled={busy} title="Filter pods by namespace">
      <option value="">All namespaces</option>
      {#each sortedNamespaces as n, i (n + '#' + i)}<option value={n}>{pinnedNs.includes(n) ? "★ " : ""}{n}</option>{/each}
    </select>
    {#if currentNs}
      <button class="pin" class:on={pinnedNs.includes(currentNs)} title={pinnedNs.includes(currentNs) ? "Unpin namespace" : "Pin namespace to top"} onclick={() => togglePinNs(currentNs)}>★</button>
    {/if}
    <span class="spacer"></span>
    {#if busy}<span class="spin">…</span>{/if}
    <button class="iconbtn" onclick={openRollouts} title="Rollout status (deployments READY/UP-TO-DATE)">
      <Icon name="workspace" size={13} />
    </button>
    <button class="iconbtn" onclick={openNodes} title="Node capacity & usage (kubectl top nodes)">
      <Icon name="chart" size={13} />
    </button>
    <button class="iconbtn" onclick={() => { load(); refreshPods(); refreshPf(); fluxNonce++; }} title="Refresh" disabled={busy}>
      <Icon name="refresh" size={13} />
    </button>
  </div>

  <!-- Auth error bar. Hidden in the Flux view — that error comes from the pod
       watcher; Flux shows its own per-tab error so a stale pod failure doesn't
       paint a false banner over a healthy Flux list. -->
  {#if authErr && view !== "flux"}
    <div class="authbar">
      <Icon name="alert" size={13} />
      <span>{cloudInfo?.profile ? `Sign in to AWS SSO for "${cloudInfo.profile}"` : "Cloud credentials expired or missing."}</span>
      {#each reauthActions(current, cloudInfo ?? undefined) as a, i (a.cmd + '#' + i)}
        <button onclick={() => onRunCommand?.(a.cmd)}>{a.label}</button>
      {/each}
      <button class="ghost" onclick={() => { load(); refreshPods(); fluxNonce++; }}>Retry</button>
      {#if onCheckConnections}<button class="ghost" onclick={onCheckConnections}>Check connections</button>{/if}
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
    <Flux {onRunCommand} onPresence={onFluxPresence} {onHealth} {onInvestigate} {active} visible={active && view === "flux"} refreshNonce={fluxNonce} />
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
      {#each pfList as f, i (f.pid + '#' + i)}
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
    <div class="pods" class:split={!!panel} style={panel ? `flex:0 0 ${podsW}px` : ""}>
      {#if conn === "auth"}
        <EmptyState icon="kube" title="Not connected" hint={`Authenticate to "${current}" above — nothing is shown until this cluster is connected.`} />
      {:else if conn === "error"}
        <div class="empty" title={k8sErr}>{friendlyErr(k8sErr)}</div>
      {:else if conn === "connecting"}
        <Skeleton rows={10} />
      {:else if podRows.length}
        <div class="pod-summary">
          <span class="sum-chip ok"><i></i>{podStats.running} running</span>
          {#if podStats.pending}<span class="sum-chip warn"><i></i>{podStats.pending} pending</span>{/if}
          {#if podStats.failing}<span class="sum-chip bad"><i></i>{podStats.failing} failing</span>{/if}
          <span class="spacer"></span>
          <span class="sum-total" title={current}>{podStats.total} pods · {current.split('/').pop()}</span>
        </div>
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
        {#each shownPods as p (`${p.ns}/${p.name}`)}
          <div class="pod-row" role="button" tabindex="0"
            onclick={() => openLogs(p)}
            onkeydown={(e) => e.key === "Enter" && openLogs(p)}
            title="View logs"
          >
            <span class="col-dot">
              <span class="dot {statusKind(p.status)}"></span>
            </span>
            <span class="col-name mono">{p.name}</span>
            <span class="col-ready muted">{p.ready}</span>
            <span class="col-restarts" style="color:{Number(p.restarts) > 0 ? 'var(--red)' : 'var(--text3)'}">
              {p.restarts}
            </span>
            <span class="col-status"><span class="st-chip {statusKind(p.status)}">{p.status}</span></span>
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
      {:else}
        <EmptyState icon="kube" title="No pods" hint="This cluster has no pods in the selected namespace." />
      {/if}
    </div>

    <!-- Inline log/describe panel -->
    {#if panel}
      <Resizer bind:size={podsW} min={320} max={1100} def={640} storeKey="anvil-kube-podsw" />
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
  /* Opt-in shell-context sync: shown only when Anvil is viewing a different
     cluster than the shell. Coral = trace/active operational state (brand). */
  .shellsync {
    display: inline-flex; align-items: center; gap: 4px; flex: 0 0 auto;
    background: transparent; border: 1px solid var(--border); color: var(--text3);
    font-family: var(--font-mono); font-size: 10.5px; padding: 1px 7px;
    border-radius: 6px; cursor: default; white-space: nowrap;
  }
  .shellsync:hover { color: var(--accent); border-color: var(--accent); }

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

  /* Operational health glance — chips encode real cluster state (brand). */
  .pod-summary {
    display: flex; align-items: center; gap: 7px; padding: 7px 12px;
    border-bottom: 1px solid var(--hairline); flex: 0 0 auto;
  }
  .pod-summary .spacer { flex: 1; }
  .sum-chip {
    display: inline-flex; align-items: center; gap: 6px; font-family: var(--font-mono);
    font-size: 11px; font-weight: 500; padding: 2px 9px 2px 8px; border-radius: 11px;
    border: 1px solid var(--border); color: var(--text2); background: var(--panel2);
  }
  .sum-chip i { width: 6px; height: 6px; border-radius: 50%; flex: 0 0 auto; background: var(--text3); }
  .sum-chip.ok i { background: var(--status-verified, var(--green)); box-shadow: 0 0 5px color-mix(in srgb, var(--status-verified, var(--green)) 70%, transparent); }
  .sum-chip.warn i { background: var(--status-attention, var(--yellow)); }
  .sum-chip.bad { color: var(--status-failure, var(--red)); border-color: color-mix(in srgb, var(--status-failure, var(--red)) 45%, var(--border)); }
  .sum-chip.bad i { background: var(--status-failure, var(--red)); box-shadow: 0 0 5px color-mix(in srgb, var(--status-failure, var(--red)) 70%, transparent); }
  .sum-total { font-family: var(--font-mono); font-size: 10.5px; color: var(--text3);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 50%; }

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
  /* Live status dot — sparse glow only on running (live) + failing (attention). */
  .dot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; background: var(--text3); }
  .dot.ok { background: var(--status-verified, var(--green)); box-shadow: 0 0 5px color-mix(in srgb, var(--status-verified, var(--green)) 75%, transparent); }
  .dot.warn { background: var(--status-attention, var(--yellow)); }
  .dot.bad { background: var(--status-failure, var(--red)); box-shadow: 0 0 5px color-mix(in srgb, var(--status-failure, var(--red)) 75%, transparent); }
  .col-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
  .col-ready { text-align: right; }
  .col-restarts { text-align: right; font-family: var(--font-mono); }
  .col-status { text-align: right; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* Status chip — squared pill, semantic color, mono label (brand). */
  .st-chip {
    display: inline-block; font-family: var(--font-mono); font-size: 10px; font-weight: 500;
    letter-spacing: 0.01em; padding: 1px 7px; border-radius: 4px; max-width: 100%;
    overflow: hidden; text-overflow: ellipsis; vertical-align: middle;
    color: var(--text3); background: color-mix(in srgb, var(--text) 7%, transparent);
  }
  .st-chip.ok { color: var(--status-verified, var(--green)); background: color-mix(in srgb, var(--status-verified, var(--green)) 13%, transparent); }
  .st-chip.warn { color: var(--status-attention, var(--yellow)); background: color-mix(in srgb, var(--status-attention, var(--yellow)) 13%, transparent); }
  .st-chip.bad { color: var(--status-failure, var(--red)); background: color-mix(in srgb, var(--status-failure, var(--red)) 14%, transparent); }
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
