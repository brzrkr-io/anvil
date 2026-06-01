<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { get } from "svelte/store";
  import { online } from "$lib/offline";
  const offlineMsg = "Offline — reconnect to query cluster / observability.";
  import { ACCOUNTS, getValue } from "$lib/accounts";
  import Icon from "$lib/Icon.svelte";

  let { cwd, onRunCommand }: { cwd: string; onRunCommand?: (cmd: string) => void } = $props();

  let tab = $state<"k8s" | "ci" | "prs" | "obs" | "tf" | "helm" | "gitlab" | "inc" | "aws">("k8s");
  // #59 AWS in-pane resource browser.
  let awsSvc = $state<"ec2" | "s3" | "lambda" | "rds">("ec2");
  let awsOut = $state("");
  let awsBusy = $state(false);
  async function loadAws() {
    awsBusy = true; awsOut = "Loading…";
    try { awsOut = await invoke<string>("aws_list", { service: awsSvc }); }
    catch (e) { awsOut = String(e); }
    awsBusy = false;
  }
  let tfPlan = $state("");
  let tfBusy = $state(false);
  async function runTfPlan() {
    tfBusy = true;
    try { tfPlan = await invoke<string>("terraform_plan", { cwd }); }
    catch (e) { tfPlan = String(e); }
    tfBusy = false;
  }
  async function runTfState() {
    tfBusy = true;
    try { tfPlan = await invoke<string>("terraform_state", { cwd }); }
    catch (e) { tfPlan = String(e); }
    tfBusy = false;
  }
  async function runTfApply() {
    // Approval gate (#52): irreversible infra change — explicit confirm required.
    if (!confirm(`Run terraform apply -auto-approve in ${cwd.split("/").pop()}?\n\nThis applies the plan and may create/destroy real infrastructure.`)) return;
    tfBusy = true;
    try { tfPlan = await invoke<string>("terraform_apply", { cwd }); }
    catch (e) { tfPlan = String(e); }
    tfBusy = false;
  }
  const tfClass = (ln: string) => {
    const t = ln.trimStart();
    if (t.startsWith("+/-") || t.startsWith("-/+")) return "rep";
    if (t.startsWith("+")) return "add";
    if (t.startsWith("-")) return "del";
    if (t.startsWith("~")) return "chg";
    return "";
  };
  let contexts = $state<string[]>([]);
  let current = $state("");
  let namespaces = $state<string[]>([]);
  let currentNs = $state("default");
  let logSelector = $state("");
  let pods = $state("");
  let logPod = $state<{ ns: string; name: string } | null>(null);
  let logs = $state("");
  let runs = $state("");

  // Parse `kubectl get pods -A` into clickable rows (NAMESPACE NAME READY STATUS …).
  interface Pod { ns: string; name: string; ready: string; status: string; }
  const podRows = $derived.by<Pod[]>(() => {
    const lines = pods.split("\n").filter(Boolean);
    if (!lines.length || !/^NAMESPACE\s/.test(lines[0])) return [];
    return lines.slice(1).map((l) => {
      const c = l.split(/\s+/);
      return { ns: c[0] ?? "", name: c[1] ?? "", ready: c[2] ?? "", status: c[3] ?? "" };
    }).filter((p) => p.name);
  });
  async function openLogs(p: Pod) {
    logPod = { ns: p.ns, name: p.name };
    logs = "Loading…";
    try { logs = await invoke<string>("kube_logs", { context: current, namespace: p.ns, pod: p.name }); }
    catch (e) { logs = String(e); }
  }
  async function multiplexLogs() {
    if (!logSelector) return;
    logPod = { ns: currentNs, name: `-l ${logSelector}` };
    logs = "Loading…";
    try { logs = await invoke<string>("kube_logs_selector", { context: current, namespace: currentNs, selector: logSelector }); }
    catch (e) { logs = String(e); }
  }
  async function describePod(p: Pod) {
    logPod = { ns: p.ns, name: p.name };
    logs = "Loading…";
    try { logs = await invoke<string>("kube_describe", { context: current, namespace: p.ns, pod: p.name }); }
    catch (e) { logs = String(e); }
  }
  async function deletePod(p: Pod) {
    if (!confirm(`Delete pod ${p.name}? (its controller will recreate it)`)) return;
    try { logs = await invoke<string>("kube_delete_pod", { context: current, namespace: p.ns, pod: p.name }); logPod = { ns: p.ns, name: p.name }; await loadK8s(); }
    catch (e) { logs = String(e); }
  }
  async function restartPod(p: Pod) {
    const dep = p.name.replace(/-[a-z0-9]+-[a-z0-9]+$/, "");
    if (!confirm(`kubectl rollout restart deployment/${dep} in ${p.ns}?`)) return;
    try { logs = await invoke<string>("kube_restart", { context: current, namespace: p.ns, deployment: dep }); logPod = { ns: p.ns, name: p.name }; }
    catch (e) { logs = String(e); }
  }
  const statusColor = (s: string) =>
    s === "Running" || s === "Completed" ? "var(--green)"
      : /Error|CrashLoop|Failed|Evicted/.test(s) ? "var(--red)"
      : "var(--yellow)";
  let prs = $state("");
  let obsUrl = $state(typeof localStorage !== "undefined" ? localStorage.getItem("anvil-obs-url") ?? "" : "");
  // Saved dashboards / deep-links (#79), persisted in localStorage.
  function loadDash(): { name: string; url: string }[] {
    try { return JSON.parse(localStorage.getItem("anvil-dashboards") || "[]"); } catch { return []; }
  }
  let savedDashboards = $state<{ name: string; url: string }[]>(typeof localStorage !== "undefined" ? loadDash() : []);
  function persistDash() { if (typeof localStorage !== "undefined") localStorage.setItem("anvil-dashboards", JSON.stringify(savedDashboards)); }
  function saveDash() {
    if (!obsUrl) return;
    const name = prompt("Name this dashboard:", obsUrl.split("/").pop() || "dashboard");
    if (!name) return;
    savedDashboards = [...savedDashboards.filter((d) => d.url !== obsUrl), { name, url: obsUrl }];
    persistDash();
  }
  function removeDash(url: string) { savedDashboards = savedDashboards.filter((d) => d.url !== url); persistDash(); }

  // Native Prometheus instant query (#77) — no iframe.
  let promBase = $state(typeof localStorage !== "undefined" ? localStorage.getItem("anvil-prom-base") ?? "" : "");
  let promQuery = $state("");
  let promRows = $state<{ metric: string; value: string }[]>([]);
  let promErr = $state("");
  async function runProm() {
    if (!promBase || !promQuery) return;
    if (!get(online)) { promErr = offlineMsg; return; }
    if (typeof localStorage !== "undefined") localStorage.setItem("anvil-prom-base", promBase);
    promErr = ""; promRows = [];
    try {
      const j = JSON.parse(await invoke<string>("prom_query", { base: promBase, query: promQuery }));
      if (j.status !== "success") { promErr = j.error || "query error"; return; }
      promRows = (j.data?.result ?? []).map((r: any) => ({
        metric: Object.entries(r.metric ?? {}).map(([k, v]) => `${k}="${v}"`).join(", ") || "{}",
        value: Array.isArray(r.value) ? String(r.value[1]) : "",
      }));
      if (!promRows.length) promErr = "no matching series";
    } catch (e) { promErr = String(e); }
  }
  // Saved Prometheus queries + sparklines (#55), persisted in localStorage.
  function loadPromQs(): { name: string; q: string }[] {
    try { return JSON.parse(localStorage.getItem("anvil-prom-queries") || "[]"); } catch { return []; }
  }
  let savedPromQs = $state<{ name: string; q: string }[]>(typeof localStorage !== "undefined" ? loadPromQs() : []);
  let sparks = $state<Record<string, number[]>>({});
  function persistPromQs() { if (typeof localStorage !== "undefined") localStorage.setItem("anvil-prom-queries", JSON.stringify(savedPromQs)); }
  function savePromQuery() {
    if (!promQuery.trim()) return;
    const name = prompt("Name this query:", promQuery.slice(0, 40));
    if (!name) return;
    savedPromQs = [...savedPromQs.filter((x) => x.q !== promQuery), { name, q: promQuery }];
    persistPromQs();
    loadSparks();
  }
  function removePromQuery(q: string) { savedPromQs = savedPromQs.filter((x) => x.q !== q); persistPromQs(); delete sparks[q]; }
  async function loadSparks() {
    if (!promBase || !savedPromQs.length) return;
    for (const { q } of savedPromQs) {
      try {
        const j = JSON.parse(await invoke<string>("prom_query_range", { base: promBase, query: q, minutes: 60 }));
        const res = j.data?.result?.[0];
        sparks[q] = (res?.values ?? []).map((v: any) => parseFloat(v[1])).filter((n: number) => !Number.isNaN(n));
      } catch { sparks[q] = []; }
    }
  }
  function sparkLast(vals: number[]): string {
    if (!vals?.length) return "—";
    const n = vals[vals.length - 1];
    return Math.abs(n) >= 1000 || (n !== 0 && Math.abs(n) < 0.01) ? n.toExponential(2) : n.toFixed(2);
  }
  function sparkPath(vals: number[], w = 120, h = 22): string {
    if (!vals || vals.length < 2) return "";
    const min = Math.min(...vals), max = Math.max(...vals), span = max - min || 1;
    return vals.map((v, i) => `${i === 0 ? "M" : "L"}${((i / (vals.length - 1)) * w).toFixed(1)} ${(h - ((v - min) / span) * h).toFixed(1)}`).join(" ");
  }
  $effect(() => { if (tab === "obs" && promBase && savedPromQs.length) loadSparks(); });
  $effect(() => { if (tab === "obs" && promBase) loadAlerts(); }); // #57 inline alerts

  // #61 Unified secrets read (SSM / Vault / Keychain) — value never persisted.
  let secSource = $state<"ssm" | "vault" | "keychain">("ssm");
  let secKey = $state("");
  let secVal = $state("");
  let secErr = $state("");
  let secReveal = $state(false);
  async function readSecret() {
    if (!secKey.trim()) return;
    secErr = ""; secVal = ""; secReveal = false;
    try { secVal = await invoke<string>("secret_read", { source: secSource, key: secKey.trim() }); }
    catch (e) { secErr = String(e).slice(0, 200); }
  }

  // Incident mode (#60) — unified firing alerts + metric sparklines, one view.
  let alerts = $state<{ name: string; sev: string; labels: string }[]>([]);
  let alertErr = $state("");
  let alertLoading = $state(false);
  async function loadAlerts() {
    if (!promBase) { alertErr = "Set a Prometheus base URL in Observability first"; return; }
    alertErr = ""; alertLoading = true;
    try {
      const j = JSON.parse(await invoke<string>("prom_query", { base: promBase, query: 'ALERTS{alertstate="firing"}' }));
      if (j.status !== "success") { alertErr = j.error || "query error"; alerts = []; return; }
      alerts = (j.data?.result ?? []).map((r: any) => {
        const m = r.metric ?? {};
        return {
          name: m.alertname || "alert",
          sev: m.severity || "",
          labels: Object.entries(m).filter(([k]) => !["alertname", "alertstate", "severity", "__name__"].includes(k)).map(([k, v]) => `${k}=${v}`).join("  "),
        };
      });
    } catch (e) { alertErr = String(e); alerts = []; } finally { alertLoading = false; }
  }
  $effect(() => { if (tab === "inc" && promBase) { loadAlerts(); loadSparks(); } });

  // Native Loki LogQL query (#56).
  let lokiBase = $state(typeof localStorage !== "undefined" ? localStorage.getItem("anvil-loki-base") ?? "" : "");
  let lokiQuery = $state("");
  let lokiLines = $state<{ ts: string; line: string }[]>([]);
  let lokiErr = $state("");
  async function runLoki() {
    if (!lokiBase || !lokiQuery) return;
    if (!get(online)) { lokiErr = offlineMsg; return; }
    if (typeof localStorage !== "undefined") localStorage.setItem("anvil-loki-base", lokiBase);
    lokiErr = ""; lokiLines = [];
    try {
      const j = JSON.parse(await invoke<string>("loki_query", { base: lokiBase, query: lokiQuery }));
      if (j.status !== "success") { lokiErr = j.error || "query error"; return; }
      const out: { ts: string; line: string }[] = [];
      for (const s of j.data?.result ?? []) {
        for (const [ns, line] of s.values ?? []) {
          out.push({ ts: new Date(Number(ns) / 1e6).toLocaleTimeString(), line });
        }
      }
      out.sort((a, b) => (a.ts < b.ts ? 1 : -1));
      lokiLines = out.slice(0, 300);
      if (!lokiLines.length) lokiErr = "no log lines";
    } catch (e) { lokiErr = String(e); }
  }
  // #56 Loki live tail — poll the query every 3s while enabled.
  let lokiTail = $state(false);
  let lokiTimer: ReturnType<typeof setInterval> | null = null;
  function toggleLokiTail() {
    lokiTail = !lokiTail;
    if (lokiTimer) { clearInterval(lokiTimer); lokiTimer = null; }
    if (lokiTail) { runLoki(); lokiTimer = setInterval(() => { if (lokiTail) runLoki(); }, 3000); }
  }
  $effect(() => () => { if (lokiTimer) clearInterval(lokiTimer); });
  let busy = $state(false);

  // Detect expired/invalid cloud creds in any panel output so we can offer a
  // one-click re-auth instead of a wall of errors.
  const AUTH_RE = /expired|credentials|unauthorized|not logged in|sso session|reauthenticate|InvalidIdentityToken|token has expired|failed to get token/i;
  const k8sAuthErr = $derived(AUTH_RE.test(pods));
  const ciAuthErr = $derived(AUTH_RE.test(runs) || AUTH_RE.test(prs));
  function runCmd(cmd: string) { onRunCommand?.(cmd); }
  function execPod(p: { ns: string; name: string }) {
    runCmd(`kubectl exec -it -n ${p.ns} ${p.name} -- sh -c 'command -v bash >/dev/null && exec bash || exec sh'`);
  }
  // #48 Managed port-forwards: spawn a tracked child, list + stop in-pane.
  let pfList = $state<{ pid: string; desc: string }[]>([]);
  let k8sErr = $state("");
  async function refreshPf() {
    try { pfList = (await invoke<string>("kube_pf_list")).split("\n").filter(Boolean).map((l) => { const [pid, desc] = l.split("\t"); return { pid, desc }; }); }
    catch { pfList = []; }
  }
  async function portForwardPod(p: { ns: string; name: string }) {
    const ports = prompt(`Port-forward ${p.name} (local:remote, e.g. 8080:80):`);
    if (!ports || !/^\d+:\d+$/.test(ports.trim())) return;
    try { await invoke("kube_pf_start", { context: current, namespace: p.ns, pod: p.name, ports: ports.trim() }); await refreshPf(); }
    catch (e) { k8sErr = String(e); }
  }
  async function stopPf(pid: string) { try { await invoke("kube_pf_stop", { pid: Number(pid) }); } catch { /* ignore */ } await refreshPf(); }

  async function loadK8s() {
    busy = true;
    try {
      contexts = (await invoke<string>("kube_contexts")).split("\n").filter(Boolean);
      current = (await invoke<string>("kube_current_context")).trim();
      try { currentNs = (await invoke<string>("kube_current_namespace")).trim() || "default"; } catch { currentNs = "default"; }
      try { namespaces = (await invoke<string>("kube_namespaces")).split("\n").filter(Boolean); } catch { namespaces = []; }
      pods = await invoke<string>("kube_pods", { context: current });
    } catch (e) { pods = String(e); }
    busy = false;
  }
  async function useCtx(name: string) {
    if (!name || name === current) return;
    busy = true;
    try { await invoke("kube_use_context", { name }); current = name; await loadK8s(); }
    catch (e) { pods = String(e); }
    busy = false;
  }
  async function useNs(ns: string) {
    if (!ns || ns === currentNs) return;
    busy = true;
    try { await invoke("kube_set_namespace", { namespace: ns }); currentNs = ns; }
    catch (e) { pods = String(e); }
    busy = false;
  }
  interface Run { databaseId: number; status: string; conclusion: string; displayTitle: string; workflowName: string; headBranch: string; event: string; }
  let runRows = $state<Run[]>([]);
  async function loadCI() {
    busy = true;
    try {
      const j = await invoke<string>("gh_runs_json", { cwd });
      runRows = JSON.parse(j);
      runs = "";
    } catch (e) {
      runRows = [];
      try { runs = await invoke<string>("gh_runs", { cwd }); } catch { runs = String(e); }
    }
    busy = false;
  }
  async function rerun(id: number) {
    busy = true;
    try { await invoke("gh_rerun", { cwd, id: String(id) }); } catch (e) { runs = String(e); }
    await loadCI();
  }
  let runLog = $state<{ id: number; title: string } | null>(null);
  let runLogText = $state("");
  async function viewRunLog(r: Run) {
    runLog = { id: r.databaseId, title: r.displayTitle };
    runLogText = "Loading…";
    try { runLogText = await invoke<string>("gh_run_log", { cwd, id: String(r.databaseId) }); }
    catch (e) { runLogText = String(e); }
  }
  const runColor = (r: Run) =>
    r.status !== "completed" ? "var(--yellow)"
      : r.conclusion === "success" ? "var(--green)"
      : "var(--red)";
  // #27 PR review: open a PR's body + comments inline, post a comment.
  let prSel = $state("");
  let prDetail = $state("");
  let prComment = $state("");
  let prBusy = $state(false);
  const prRows = $derived(prs.split("\n").filter(Boolean).map((l) => { const p = l.split("\t"); return { num: p[0], title: p[1] || p[0] }; }).filter((r) => /^\d+$/.test(r.num)));
  async function openPr(num: string) {
    prSel = num; prDetail = "Loading…";
    try { prDetail = await invoke<string>("gh_pr_view", { cwd, num }); } catch (e) { prDetail = String(e); }
  }
  async function postPrComment() {
    if (!prSel || !prComment.trim() || prBusy) return;
    prBusy = true;
    try { await invoke("gh_pr_comment", { cwd, num: prSel, body: prComment }); prComment = ""; await openPr(prSel); } catch (e) { prDetail = String(e) + "\n\n" + prDetail; }
    prBusy = false;
  }
  async function loadPRs() {
    busy = true;
    try { prs = await invoke<string>("gh_prs", { cwd }); } catch (e) { prs = String(e); }
    busy = false;
  }
  interface Helm { name: string; namespace: string; revision: string; status: string; chart: string; app_version: string; }
  let helmRows = $state<Helm[]>([]);
  let helmErr = $state("");
  let helmValues = $state<{ name: string; text: string } | null>(null);
  async function loadHelm() {
    busy = true; helmErr = "";
    try { helmRows = JSON.parse(await invoke<string>("helm_list")); }
    catch (e) { helmErr = String(e); helmRows = []; }
    busy = false;
  }
  let helmAllValues = $state(false);
  let helmCur = $state<Helm | null>(null);
  async function showValues(h: Helm) {
    helmCur = h;
    helmValues = { name: `${h.namespace}/${h.name}`, text: "Loading…" };
    const cmd = helmAllValues ? "helm_values_all" : "helm_values";
    try { helmValues = { name: `${h.namespace}/${h.name}`, text: await invoke<string>(cmd, { name: h.name, namespace: h.namespace }) }; }
    catch (e) { helmValues = { name: `${h.namespace}/${h.name}`, text: String(e) }; }
  }
  function toggleHelmAll() { helmAllValues = !helmAllValues; if (helmCur) showValues(helmCur); }
  // GitLab CI (#54) — via the authed glab CLI, run in the repo cwd.
  let glabOut = $state("");
  async function loadGlab() {
    busy = true;
    try { glabOut = await invoke<string>("glab_pipelines", { cwd }); }
    catch (e) { glabOut = String(e); }
    busy = false;
  }
  function refresh() {
    if (tab === "k8s") { loadK8s(); refreshPf(); }
    else if (tab === "ci") loadCI();
    else if (tab === "prs") loadPRs();
  }
  function saveObs() {
    if (typeof localStorage !== "undefined") localStorage.setItem("anvil-obs-url", obsUrl);
  }

  // Apply AWS profile + GitHub token from Accounts so kubectl / gh use them.
  async function applyCreds() {
    const aws = ACCOUNTS.find((a) => a.key === "aws-profile");
    const gh = ACCOUNTS.find((a) => a.key === "github-token");
    try { if (aws) await invoke("set_aws_profile", { profile: await getValue(aws) }); } catch { /* ignore */ }
    try { if (gh) await invoke("set_github_token", { token: await getValue(gh) }); } catch { /* ignore */ }
  }
  onMount(async () => { await applyCreds(); loadK8s(); loadCI(); });
  $effect(() => { if (tab === "prs" && !prs) loadPRs(); });
</script>

<div class="dev">
  <div class="tabs">
    <button class:on={tab === "k8s"} onclick={() => (tab = "k8s")}><Icon name="devops" size={14} /> Kubernetes</button>
    <button class:on={tab === "ci"} onclick={() => (tab = "ci")}><Icon name="ci" size={14} /> CI Runs</button>
    <button class:on={tab === "prs"} onclick={() => (tab = "prs")}><Icon name="pr" size={14} /> Pull Requests</button>
    <button class:on={tab === "obs"} onclick={() => (tab = "obs")}><Icon name="chart" size={14} /> Observability</button>
    <button class:on={tab === "inc"} onclick={() => (tab = "inc")}><Icon name="alert" size={14} /> Incident</button>
    <button class:on={tab === "aws"} onclick={() => { tab = "aws"; if (!awsOut) loadAws(); }}><Icon name="devops" size={14} /> AWS</button>
    <button class:on={tab === "tf"} onclick={() => (tab = "tf")}><Icon name="devops" size={14} /> Terraform</button>
    <button class:on={tab === "helm"} onclick={() => { tab = "helm"; if (!helmRows.length) loadHelm(); }}><Icon name="devops" size={14} /> Helm</button>
    <button class:on={tab === "gitlab"} onclick={() => { tab = "gitlab"; if (!glabOut) loadGlab(); }}><Icon name="ci" size={14} /> GitLab CI</button>
    <span class="sp"></span>
    {#if busy}<span class="busy">…</span>{/if}
    {#if tab !== "obs"}<button class="refresh" onclick={refresh} title="Refresh"><Icon name="refresh" size={13} /></button>{/if}
  </div>

  {#if k8sAuthErr && tab === "k8s"}
    <div class="authbar">
      <span class="aw">⚠ Cloud credentials expired or missing.</span>
      <button onclick={() => runCmd("aws sso login")}>aws sso login</button>
      <button onclick={() => runCmd(`aws eks update-kubeconfig --name "$(kubectl config current-context)"`)}>refresh kubeconfig</button>
      <button class="ghost" onclick={loadK8s}>Retry</button>
    </div>
  {/if}
  {#if ciAuthErr && (tab === "ci" || tab === "prs")}
    <div class="authbar">
      <span class="aw">⚠ GitHub CLI not authenticated.</span>
      <button onclick={() => runCmd("gh auth login")}>gh auth login</button>
      <button class="ghost" onclick={refresh}>Retry</button>
    </div>
  {/if}

  {#if tab === "k8s"}
    {#if k8sErr}<div class="empty">{k8sErr}</div>{/if}
    {#if pfList.length}
      <div class="bar"><span class="lbl">Port-forwards</span>
        {#each pfList as f (f.pid)}<span class="rfeat" style="text-transform:none">{f.desc} <button class="pfx" onclick={() => stopPf(f.pid)} title="Stop">✕</button></span>{/each}
      </div>
    {/if}
    <div class="bar">
      <span class="lbl">Context</span>
      <select value={current} onchange={(e) => useCtx((e.currentTarget as HTMLSelectElement).value)}>
        {#each contexts as c (c)}<option value={c}>{c}</option>{/each}
      </select>
      <span class="lbl">Namespace</span>
      <select value={currentNs} onchange={(e) => useNs((e.currentTarget as HTMLSelectElement).value)}>
        {#if !namespaces.includes(currentNs)}<option value={currentNs}>{currentNs}</option>{/if}
        {#each namespaces as n (n)}<option value={n}>{n}</option>{/each}
      </select>
      <input class="sel" bind:value={logSelector} placeholder="-l app=…" spellcheck="false" style="width:130px" />
      <button class="refresh" title="Multiplex logs across matching pods (in-pane)" disabled={!logSelector} onclick={multiplexLogs}>logs ⇉</button>
      <button class="refresh" title="Follow logs across matching pods (in terminal)" disabled={!logSelector} onclick={() => runCmd(`kubectl logs -l ${logSelector} -n ${currentNs} --all-containers --prefix --tail=200 -f`)}>tail ↗</button>
      {#if logPod}<button class="back" onclick={() => (logPod = null)}>← Pods</button>{/if}
    </div>
    {#if logPod}
      <div class="bar"><span class="lbl">Logs</span><span class="podname">{logPod.ns}/{logPod.name}</span></div>
      <pre class="out">{logs}</pre>
    {:else if podRows.length}
      <div class="podlist">
        {#each podRows as p (p.ns + "/" + p.name)}
          <div class="podrow" role="button" tabindex="0" onclick={() => openLogs(p)} title="View logs">
            <span class="pdot" style="background:{statusColor(p.status)}"></span>
            <span class="pns">{p.ns}</span>
            <span class="pnm">{p.name}</span>
            <span class="prd">{p.ready}</span>
            <span class="pst" style="color:{statusColor(p.status)}">{p.status}</span>
            <span class="pacts">
              <button class="pact" title="Describe" onclick={(e) => { e.stopPropagation(); describePod(p); }}>ⓘ</button>
              <button class="pact" title="Exec shell" onclick={(e) => { e.stopPropagation(); execPod(p); }}>›_</button>
              <button class="pact" title="Port-forward" onclick={(e) => { e.stopPropagation(); portForwardPod(p); }}>⇄</button>
              <button class="pact" title="Rollout restart deployment" onclick={(e) => { e.stopPropagation(); restartPod(p); }}>⟳</button>
              <button class="pact danger" title="Delete pod" onclick={(e) => { e.stopPropagation(); deletePod(p); }}>✕</button>
            </span>
          </div>
        {/each}
      </div>
    {:else}
      <pre class="out">{pods || "No pods / kubectl unavailable."}</pre>
    {/if}
  {:else if tab === "ci"}
    <div class="bar"><span class="lbl">CI · {cwd.split("/").pop()}</span>
      {#if runLog}<button class="back" onclick={() => (runLog = null)}>← Runs</button>{/if}
    </div>
    {#if runLog}
      <div class="bar"><span class="lbl">Log</span><span class="podname">{runLog.title}</span></div>
      <pre class="out">{runLogText}</pre>
    {:else if runRows.length}
      <div class="podlist">
        {#each runRows as r (r.databaseId)}
          <div class="podrow" role="button" tabindex="0" onclick={() => viewRunLog(r)} title="View logs">
            <span class="pdot" style="background:{runColor(r)}"></span>
            <span class="pnm">{r.displayTitle}</span>
            <span class="pns" style="width:120px">{r.workflowName}</span>
            <span class="prd">{r.headBranch}</span>
            <span class="pst" style="color:{runColor(r)}">{r.status === "completed" ? r.conclusion : r.status}</span>
            <button class="rerun" onclick={(e) => { e.stopPropagation(); rerun(r.databaseId); }} title="Re-run">↻</button>
          </div>
        {/each}
      </div>
    {:else}
      <pre class="out">{runs || "No runs / gh unavailable."}</pre>
    {/if}
  {:else if tab === "prs"}
    <div class="bar"><span class="lbl">Open PRs · {cwd.split("/").pop()}</span></div>
    {#if prRows.length}
      <div class="podlist">
        {#each prRows as r (r.num)}
          <div class="podrow" class:cur={prSel === r.num} role="button" tabindex="0" onclick={() => openPr(r.num)}>
            <span class="bdg" style="color:var(--accent)">#{r.num}</span><span class="pnm">{r.title}</span>
          </div>
        {/each}
      </div>
      {#if prSel}
        <pre class="out">{prDetail}</pre>
        <div class="bar">
          <input class="url" bind:value={prComment} onkeydown={(e) => e.key === "Enter" && postPrComment()} placeholder={`Comment on #${prSel} (Enter to post)`} spellcheck="false" />
          <button class="refresh" disabled={prBusy || !prComment.trim()} onclick={postPrComment} title="Post comment"><Icon name="play" size={13} /></button>
        </div>
      {/if}
    {:else}
      <pre class="out">{prs || "No open PRs / gh unavailable."}</pre>
    {/if}
  {:else if tab === "tf"}
    <div class="bar">
      <span class="lbl">Terraform · {cwd.split("/").pop()}</span>
      <button class="refresh" onclick={runTfPlan} title="terraform plan">Plan</button>
      <button class="refresh" onclick={runTfState} title="terraform state list">State</button>
      <button class="refresh danger" title="Apply with confirm (auto-approve after OK)" onclick={runTfApply} disabled={tfBusy}>Apply ✓</button>
      <button class="refresh" title="Apply interactively in terminal" onclick={() => runCmd("terraform apply")}>Apply ↗</button>
    </div>
    {#if tfBusy}<div class="empty">Planning…</div>{/if}
    {#if tfPlan}
      <pre class="out tf">{#each tfPlan.split("\n") as ln, i (i)}<span class="tfl {tfClass(ln)}">{ln}
</span>{/each}</pre>
    {:else if !tfBusy}
      <div class="empty">Run <b>Plan</b> to preview changes (terraform plan).</div>
    {/if}
  {:else if tab === "gitlab"}
    <div class="bar"><span class="lbl">GitLab CI · {cwd.split("/").pop()}</span>
      <button class="refresh" onclick={loadGlab} title="Refresh (glab ci list)"><Icon name="refresh" size={13} /></button>
      <button class="refresh" title="Trace latest pipeline in terminal" onclick={() => runCmd("glab ci trace")}>trace</button>
      <button class="refresh" title="Retry pipeline in terminal" onclick={() => runCmd("glab ci retry")}>retry</button>
    </div>
    <pre class="out">{glabOut || "Loading… (needs glab + a GitLab remote)"}</pre>
  {:else if tab === "helm"}
    <div class="bar"><span class="lbl">Helm releases</span>
      {#if helmValues}<button class="back" onclick={() => (helmValues = null)}>← Releases</button>{/if}
      <button class="refresh" onclick={loadHelm} title="Refresh"><Icon name="refresh" size={13} /></button>
    </div>
    {#if helmErr}<div class="empty">{helmErr}</div>{/if}
    {#if helmValues}
      <div class="bar"><span class="lbl">Values</span><span class="podname">{helmValues.name}</span><span class="sp" style="flex:1"></span><button class="refresh" class:on={helmAllValues} title="Include chart defaults (helm get values -a)" onclick={toggleHelmAll}>{helmAllValues ? "incl. defaults ✓" : "incl. defaults"}</button></div>
      <pre class="out">{helmValues.text}</pre>
    {:else if helmRows.length}
      <div class="podlist">
        {#each helmRows as h (h.namespace + "/" + h.name)}
          <div class="podrow" role="button" tabindex="0" onclick={() => showValues(h)} title="Show values">
            <span class="pdot" style="background:{h.status === 'deployed' ? 'var(--green)' : 'var(--yellow)'}"></span>
            <span class="pnm">{h.name}</span>
            <span class="pns" style="width:120px">{h.namespace}</span>
            <span class="prd">{h.chart}</span>
            <span class="pst">rev {h.revision} · {h.status}</span>
          </div>
        {/each}
      </div>
    {:else}
      <div class="empty">No releases / helm unavailable.</div>
    {/if}
  {:else if tab === "aws"}
    <div class="bar">
      <span class="lbl">AWS</span>
      <select class="sel" bind:value={awsSvc} onchange={loadAws}><option value="ec2">EC2</option><option value="s3">S3</option><option value="lambda">Lambda</option><option value="rds">RDS</option></select>
      <button class="refresh" onclick={loadAws} title="Refresh"><Icon name="refresh" size={13} /></button>
    </div>
    {#if awsBusy && !awsOut}<div class="empty">Loading…</div>{/if}
    <pre class="out">{awsOut || "Run a query."}</pre>
  {:else if tab === "inc"}
    <div class="bar">
      <span class="lbl">Firing alerts</span>
      <span class="sp" style="flex:1"></span>
      <button class="refresh" onclick={() => { loadAlerts(); loadSparks(); }} title="Refresh"><Icon name="refresh" size={13} /></button>
    </div>
    {#if alertErr}<div class="empty">{alertErr}</div>{/if}
    {#if !alertErr}
      {#if alertLoading && !alerts.length}<div class="empty">Loading…</div>
      {:else if !alerts.length}<div class="empty">No firing alerts 🎉</div>
      {:else}
        <div class="podlist">
          {#each alerts as a, i (i)}
            <div class="podrow">
              <span class="sevdot" class:crit={a.sev === "critical"} class:warn={a.sev === "warning"}></span>
              <span class="pnm">{a.name}{#if a.labels}<span class="alabels">  {a.labels}</span>{/if}</span>
              <span class="pst">{a.sev || "firing"}</span>
            </div>
          {/each}
        </div>
      {/if}
    {/if}
    <div class="bar"><span class="lbl">Key metrics</span></div>
    {#if savedPromQs.length}
      <div class="sparks">
        {#each savedPromQs as s (s.q)}
          <div class="spark" title={s.q}>
            <div class="spark-top"><span class="spark-nm">{s.name}</span><span class="spark-val">{sparkLast(sparks[s.q])}</span></div>
            <svg class="spark-svg" viewBox="0 0 120 22" preserveAspectRatio="none"><path d={sparkPath(sparks[s.q] ?? [])} fill="none" stroke="var(--accent)" stroke-width="1.5" /></svg>
          </div>
        {/each}
      </div>
    {:else}
      <div class="empty">Save PromQL queries in Observability to pin them here.</div>
    {/if}
    <div class="bar">
      <span class="lbl">Recent logs</span>
      <input class="url" bind:value={lokiQuery} onkeydown={(e) => e.key === "Enter" && runLoki()} placeholder={'LogQL — e.g. {app="api"} |= "error"  (Enter)'} spellcheck="false" />
      <button class="refresh" onclick={runLoki} title="Query Loki"><Icon name="play" size={13} /></button>
      <button class="refresh" class:on={lokiTail} onclick={toggleLokiTail} title="Live tail (3s)">{lokiTail ? "■" : "▶"}</button>
    </div>
    {#if lokiErr}<div class="empty">{lokiErr}</div>{/if}
    {#if lokiLines.length}
      <pre class="out">{#each lokiLines.slice(0, 80) as l}<span class="lokiln"><span class="lokits">{l.ts}</span> {l.line}</span>{/each}</pre>
    {/if}
  {:else}
    <div class="bar">
      <span class="lbl">Prometheus</span>
      <input class="url" bind:value={promBase} placeholder="http://localhost:9090" spellcheck="false" />
    </div>
    <div class="bar">
      <input class="url" bind:value={promQuery} onkeydown={(e) => e.key === "Enter" && runProm()} placeholder="PromQL — e.g. up  (Enter to run)" spellcheck="false" />
      <button class="refresh" onclick={runProm} title="Run query"><Icon name="play" size={13} /></button>
      {#if promQuery.trim()}<button class="refresh" onclick={savePromQuery} title="Save query">★</button>{/if}
    </div>
    {#if promErr}<div class="empty">{promErr}</div>{/if}
    {#if promRows.length}
      <div class="podlist">
        {#each promRows as r, i (i)}
          <div class="podrow"><span class="pnm">{r.metric}</span><span class="pst">{r.value}</span></div>
        {/each}
      </div>
    {/if}
    {#if savedPromQs.length}
      <div class="sparks">
        {#each savedPromQs as s (s.q)}
          <div class="spark" role="button" tabindex="0" title={s.q} onclick={() => { promQuery = s.q; runProm(); }} onkeydown={(e) => e.key === "Enter" && (promQuery = s.q, runProm())}>
            <div class="spark-top"><span class="spark-nm">{s.name}</span><span class="spark-val">{sparkLast(sparks[s.q])}</span><button class="spark-x" title="Remove" onclick={(e) => { e.stopPropagation(); removePromQuery(s.q); }}>×</button></div>
            <svg class="spark-svg" viewBox="0 0 120 22" preserveAspectRatio="none"><path d={sparkPath(sparks[s.q] ?? [])} fill="none" stroke="var(--accent)" stroke-width="1.5" /></svg>
          </div>
        {/each}
      </div>
    {/if}
    <div class="bar">
      <span class="lbl">Loki</span>
      <input class="url" bind:value={lokiBase} placeholder="http://localhost:3100" spellcheck="false" />
    </div>
    <div class="bar">
      <input class="url" bind:value={lokiQuery} onkeydown={(e) => e.key === "Enter" && runLoki()} placeholder={'LogQL — e.g. {app="api"} |= "error"  (Enter)'} spellcheck="false" />
      <button class="refresh" onclick={runLoki} title="Run LogQL"><Icon name="play" size={13} /></button>
      <button class="refresh" class:on={lokiTail} onclick={toggleLokiTail} title="Live tail (poll every 3s)">{lokiTail ? "■ tail" : "▶ tail"}</button>
    </div>
    {#if lokiErr}<div class="empty">{lokiErr}</div>{/if}
    {#if lokiLines.length}
      <pre class="out">{#each lokiLines as l}<span class="lokiln"><span class="lokits">{l.ts}</span> {l.line}</span>{/each}</pre>
    {/if}
    <div class="bar">
      <span class="lbl">Secrets</span>
      <select class="sel" bind:value={secSource}><option value="ssm">SSM</option><option value="vault">Vault</option><option value="keychain">Keychain</option></select>
      <input class="url" bind:value={secKey} onkeydown={(e) => e.key === "Enter" && readSecret()} placeholder={secSource === "ssm" ? "/path/to/param" : secSource === "vault" ? "secret/path" : "service name"} spellcheck="false" />
      <button class="refresh" onclick={readSecret} title="Read (decrypted, not stored)"><Icon name="key" size={13} /></button>
    </div>
    {#if secErr}<div class="empty">{secErr}</div>{/if}
    {#if secVal}
      <div class="bar">
        <span class="lbl">Value</span>
        <code class="secval">{secReveal ? secVal : "•".repeat(Math.min(secVal.length, 24))}</code>
        <button class="refresh" onclick={() => (secReveal = !secReveal)} title="Reveal / hide">{secReveal ? "hide" : "show"}</button>
        <button class="refresh" onclick={() => navigator.clipboard.writeText(secVal).catch(() => {})} title="Copy">copy</button>
      </div>
    {/if}
    {#if promBase}
      <div class="bar"><span class="lbl">Firing alerts</span><span class="sp" style="flex:1"></span><button class="refresh" title="Refresh alerts" onclick={loadAlerts}><Icon name="refresh" size={13} /></button></div>
      {#if alerts.length}
        <div class="podlist">
          {#each alerts as a, i (i)}
            <div class="podrow"><span class="sevdot" class:crit={a.sev === "critical"} class:warn={a.sev === "warning"}></span><span class="pnm">{a.name}{#if a.labels}<span class="alabels">  {a.labels}</span>{/if}</span><span class="pst">{a.sev || "firing"}</span></div>
          {/each}
        </div>
      {:else}<div class="empty">{alertErr || "No firing alerts 🎉"}</div>{/if}
    {/if}
    <div class="bar">
      <span class="lbl">Dashboard URL</span>
      <input class="url" bind:value={obsUrl} onchange={saveObs} placeholder="https://grafana…/d/…" spellcheck="false" />
      {#if obsUrl}<button class="refresh" title="Save dashboard" onclick={saveDash}>★</button>
      <button class="refresh" title="Open in a window (bypasses X-Frame-Options)" onclick={() => invoke('open_url_window', { url: obsUrl }).catch(() => {})}>↗</button>{/if}
    </div>
    {#if savedDashboards.length}
      <div class="podlist">
        {#each savedDashboards as d (d.url)}
          <div class="podrow" role="button" tabindex="0" onclick={() => { obsUrl = d.url; saveObs(); }} title={d.url}>
            <span class="pdot" style="background:var(--accent)"></span>
            <span class="pnm">{d.name}</span>
            <span class="pacts">
              <button class="pact" title="Open in window" onclick={(e) => { e.stopPropagation(); invoke('open_url_window', { url: d.url }).catch(() => {}); }}>↗</button>
              <button class="pact danger" title="Remove" onclick={(e) => { e.stopPropagation(); removeDash(d.url); }}>✕</button>
            </span>
          </div>
        {/each}
      </div>
    {/if}
    {#if obsUrl}
      <iframe class="obs" src={obsUrl} title="Observability"></iframe>
      <div class="empty" style="padding:6px 12px">Blank? Grafana blocks iframing (X-Frame-Options). Use ↗ to open it in a window.</div>
    {:else}
      <div class="empty">Paste a Grafana / dashboard URL above to embed it (or ↗ to open in a window).</div>
    {/if}
  {/if}
</div>

<style>
  .dev { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .tabs { display: flex; align-items: center; gap: 4px; height: var(--head-h, 30px); flex: 0 0 auto;
    padding: 0 8px; border-bottom: 1px solid var(--border); }
  .tabs button { display: inline-flex; align-items: center; gap: 5px; border: 0; background: transparent;
    color: var(--text2); font-family: var(--font-ui); font-size: 12px; padding: 4px 9px; border-radius: 6px; cursor: default; }
  .tabs button.on { background: var(--sel); color: var(--text); }
  .tabs .refresh { color: var(--text3); }
  .tabs .sp { flex: 1; }
  .tabs .busy { color: var(--accent); }
  .bar { display: flex; align-items: center; gap: 8px; padding: 7px 12px; border-bottom: 1px solid var(--border); }
  .lbl { color: var(--text3); font-size: 11px; font-weight: 500; }
  .bar select { background: var(--panel2); color: var(--accent); border: 1px solid var(--border);
    border-radius: 6px; padding: 3px 7px; font-size: 12px; font-family: var(--font-mono); outline: 0; }
  .out { flex: 1; min-height: 0; overflow: auto; margin: 0; padding: 10px 12px;
    font-family: var(--font-mono); font-size: 11.5px; line-height: 1.45; color: var(--text2);
    white-space: pre; }
  .lokiln { display: block; white-space: pre-wrap; word-break: break-all; }
  .lokits { color: var(--text3); }
  .url { flex: 1; background: var(--bg); color: var(--text); border: 1px solid var(--border);
    border-radius: 6px; padding: 4px 8px; font-size: 12px; font-family: var(--font-mono); outline: 0; }
  .pfx { border: 0; background: transparent; color: var(--text3); cursor: default; padding: 0 2px; font-size: 11px; }
  .pfx:hover { color: var(--danger, #e5484d); }
  .secval { flex: 1; min-width: 0; font-family: var(--font-mono); font-size: 12px; color: var(--text2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sparks { display: flex; flex-direction: column; background: var(--panel); }
  .spark { display: flex; flex-direction: column; background: transparent; padding: 4px 12px 3px;
    border-bottom: 1px solid var(--border); cursor: default; }
  .spark:hover { background: color-mix(in srgb, var(--text) 6%, transparent); }
  .spark-top { display: flex; align-items: baseline; gap: 6px; min-height: 22px; }
  .spark-nm { flex: 1; font-size: 10.5px; color: var(--text3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .spark-val { font-family: var(--font-mono); font-size: 12px; color: var(--text); }
  .spark-x { border: 0; background: transparent; color: var(--text3); font-size: 13px; cursor: default; padding: 0 2px; }
  .spark-x:hover { color: var(--text); }
  .spark-svg { display: block; width: 100%; height: 18px; }
  .sevdot { flex: 0 0 auto; width: 8px; height: 8px; border-radius: 50%; background: var(--text3); margin-right: 8px; }
  .sevdot.crit { background: var(--danger, #e5484d); }
  .sevdot.warn { background: var(--warn, #f5a623); }
  .alabels { color: var(--text3); font-size: 11px; }
  .obs { flex: 1; min-height: 0; width: 100%; border: 0; background: #fff; }
  .empty { padding: 24px; color: var(--text3); }
  .authbar { display: flex; align-items: center; gap: 8px; padding: 7px 12px;
    background: color-mix(in srgb, var(--red) 14%, var(--bg)); border-bottom: 1px solid var(--border); }
  .authbar .aw { color: var(--red); font-size: 12px; margin-right: auto; }
  .authbar button { border: 1px solid var(--accent); background: var(--accent); color: var(--bg);
    font-family: var(--font-mono); font-size: 11.5px; padding: 3px 9px; border-radius: 6px; cursor: default; }
  .authbar button.ghost { background: transparent; color: var(--text2); border-color: var(--border); }
  .authbar button:hover { filter: brightness(1.08); }
  .podlist { flex: 1; min-height: 0; overflow: auto; padding: 4px 0; }
  .podrow { display: flex; align-items: center; gap: 10px; width: 100%; border: 0; background: transparent;
    padding: 5px 12px; cursor: default; text-align: left; font-family: var(--font-mono); font-size: 11.5px; }
  .podrow:hover { background: var(--panel); }
  .pdot { flex: 0 0 auto; width: 7px; height: 7px; border-radius: 50%; }
  .pns { flex: 0 0 auto; width: 130px; color: var(--text3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pnm { flex: 1; min-width: 0; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .prd { flex: 0 0 auto; color: var(--text3); }
  .pst { flex: 0 0 auto; width: 92px; text-align: right; }
  .back { border: 1px solid var(--border); background: var(--panel2); color: var(--accent);
    font-size: 11px; padding: 2px 8px; border-radius: 6px; cursor: default; margin-left: auto; }
  .podname { color: var(--text); font-family: var(--font-mono); font-size: 12px; }
  .rerun { flex: 0 0 auto; border: 1px solid var(--border); background: transparent; color: var(--accent);
    font-size: 12px; width: 22px; height: 20px; border-radius: 6px; cursor: default; }
  .podrow:hover .rerun { background: var(--sel); }
  .pacts { flex: 0 0 auto; display: inline-flex; gap: 3px; margin-left: 6px; opacity: 0; transition: opacity 0.1s; }
  .podrow:hover .pacts { opacity: 1; }
  .pact { border: 1px solid var(--border); background: var(--panel2); color: var(--text2);
    font-size: 11px; min-width: 20px; height: 19px; border-radius: 5px; cursor: default; padding: 0 4px; }
  .pact:hover { color: var(--text); border-color: var(--text3); }
  .pact.danger:hover { color: var(--red); border-color: var(--red); }
  .out.tf { white-space: pre; }
  .tfl.add { color: var(--green); }
  .tfl.del { color: var(--red); }
  .tfl.chg { color: var(--yellow); }
  .tfl.rep { color: var(--purple); }
</style>
