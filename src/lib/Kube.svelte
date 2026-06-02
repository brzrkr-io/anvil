<script lang="ts">
  import { onMount } from "svelte";
  import { readCache, writeCache } from "$lib/cache";
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import { toast } from "$lib/toast";
  import { askConfirm, askText } from "$lib/dialog";
  import Flux from "$lib/Flux.svelte";

  let { cwd, onRunCommand }: { cwd: string; onRunCommand?: (cmd: string) => void } = $props();

  let contexts = $state<string[]>([]);
  let current = $state("");
  let namespaces = $state<string[]>([]);
  let currentNs = $state("default");
  let pods = $state("");
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

  const podRows = $derived.by<Pod[]>(() => {
    const lines = pods.split("\n").filter(Boolean);
    if (!lines.length || !/^NAMESPACE\s/.test(lines[0])) return [];
    return lines.slice(1).map((l) => {
      const c = l.split(/\s+/);
      return {
        ns: c[0] ?? "",
        name: c[1] ?? "",
        ready: c[2] ?? "",
        status: c[3] ?? "",
        restarts: c[4] ?? "0",
        age: c[5] ?? "",
      };
    }).filter((p) => p.name);
  });

  // Filter + cap the rendered rows so a cluster with thousands of pods doesn't
  // build thousands of DOM nodes at once (the render-side freeze).
  let podFilter = $state("");
  const POD_CAP = 400;
  const filteredPods = $derived(
    podFilter.trim()
      ? podRows.filter((p) => `${p.name} ${p.ns}`.toLowerCase().includes(podFilter.toLowerCase()))
      : podRows,
  );
  const shownPods = $derived(filteredPods.slice(0, POD_CAP));

  const AUTH_RE = /expired|credentials|unauthorized|not logged in|sso session|reauthenticate|InvalidIdentityToken|token has expired|failed to get token/i;
  const authErr = $derived(AUTH_RE.test(pods) || AUTH_RE.test(k8sErr));

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
    k8sErr = "";
    // Fire all five kubectl calls at once (they're independent) instead of
    // sequentially — wall time becomes the slowest call, not the sum. pods uses
    // the current context implicitly (context: "").
    const [ctxs, cur, curNs, nss, podsOut] = await Promise.allSettled([
      invoke<string>("kube_contexts"),
      invoke<string>("kube_current_context"),
      invoke<string>("kube_current_namespace"),
      invoke<string>("kube_namespaces"),
      invoke<string>("kube_pods", { context: "" }),
    ]);
    contexts = ctxs.status === "fulfilled" ? ctxs.value.split("\n").filter(Boolean) : [];
    current = cur.status === "fulfilled" ? cur.value.trim() : "";
    currentNs = curNs.status === "fulfilled" ? curNs.value.trim() || "default" : "default";
    namespaces = nss.status === "fulfilled" ? nss.value.split("\n").filter(Boolean) : [];
    if (podsOut.status === "fulfilled") { pods = podsOut.value; writeCache("kube-pods", pods); }
    else { k8sErr = String(podsOut.reason); }
    busy = false;
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
    try { await invoke("kube_use_context", { name }); current = name; await load(); }
    catch (e) { k8sErr = String(e); }
    busy = false;
  }

  async function useNs(ns: string) {
    if (!ns || ns === currentNs) return;
    busy = true;
    try { await invoke("kube_set_namespace", { namespace: ns }); currentNs = ns; await load(); }
    catch (e) { k8sErr = String(e); }
    busy = false;
  }

  async function openLogs(p: Pod) {
    panel = { pod: `${p.ns}/${p.name}`, content: "Loading…", title: "Logs" };
    try {
      const out = await invoke<string>("kube_logs", { context: current, namespace: p.ns, pod: p.name });
      panel = { pod: `${p.ns}/${p.name}`, content: out, title: "Logs" };
    } catch (e) { panel = { pod: `${p.ns}/${p.name}`, content: String(e), title: "Logs" }; }
  }

  async function openDescribe(p: Pod) {
    panel = { pod: `${p.ns}/${p.name}`, content: "Loading…", title: "Describe" };
    try {
      const out = await invoke<string>("kube_describe", { context: current, namespace: p.ns, pod: p.name });
      panel = { pod: `${p.ns}/${p.name}`, content: out, title: "Describe" };
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

  // Stream logs live in a terminal pane (the in-panel Logs view is a snapshot).
  function followLogs(p: Pod) {
    const ctx = current ? `--context ${current} ` : "";
    onRunCommand?.(`kubectl ${ctx}logs -f --tail=200 -n ${p.ns} ${p.name}`);
  }

  // Render last-known pods instantly from cache, then refresh in the background.
  onMount(() => { pods = readCache<string>("kube-pods") ?? pods; load(); refreshPf(); });
</script>

<div class="kube">
  <!-- Top bar: context, namespace, refresh -->
  <div class="topbar">
    <span class="lbl">Context</span>
    <select value={current} onchange={(e) => useCtx((e.currentTarget as HTMLSelectElement).value)} disabled={busy}>
      {#each contexts as c (c)}<option value={c}>{c}</option>{/each}
      {#if !contexts.length && current}<option value={current}>{current}</option>{/if}
    </select>
    <span class="lbl">Namespace</span>
    <select value={currentNs} onchange={(e) => useNs((e.currentTarget as HTMLSelectElement).value)} disabled={busy}>
      {#if !namespaces.includes(currentNs)}<option value={currentNs}>{currentNs}</option>{/if}
      {#each namespaces as n (n)}<option value={n}>{n}</option>{/each}
    </select>
    <span class="spacer"></span>
    {#if busy}<span class="spin">…</span>{/if}
    <button class="iconbtn" onclick={() => { load(); refreshPf(); }} title="Refresh" disabled={busy}>
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
      <button class="ghost" onclick={load}>Retry</button>
    </div>
  {/if}

  <!-- FluxCD (GitOps) — reconcile / suspend / resume / logs. Self-hides if the
       cluster has no Flux CRDs. -->
  <Flux {onRunCommand} />

  <!-- Port-forwards section -->
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
    <div class="pods" class:split={!!panel}>
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
        {#each shownPods as p (`${p.ns}/${p.name}`)}
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
              <button class="act" title="Exec shell" onclick={(e) => { e.stopPropagation(); execPod(p); }}>
                <Icon name="terminal" size={12} />
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
          {pods ? pods : "No pods found in this namespace."}
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
        <pre class="log-out">{panel.content}</pre>
      </div>
    {/if}
  </div>
</div>

<style>
  .kube { display: flex; flex-direction: column; height: 100%; min-height: 0; }

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
</style>
