<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { get } from "svelte/store";
  import { online } from "$lib/offline";
  const offlineMsg = "Offline — reconnect to query cluster / observability.";
  import { ACCOUNTS, getValue } from "$lib/accounts";
  import Icon from "$lib/Icon.svelte";
  import { toast } from "$lib/toast";
  import { askText, askConfirm } from "$lib/dialog";
  import { parsePrRows, type PrRow } from "$lib/pr-checks";
  import { githubInvestigation, actionsInvestigation } from "$lib/agent-ops";
  import { parseRuns, failingRuns, type RunRow } from "$lib/actions-runs";
  import Docker from "$lib/Docker.svelte";

  let { cwd, onRunCommand, onInvestigate }: { cwd: string; onRunCommand?: (cmd: string) => void; onInvestigate?: (prompt: string) => void } = $props();

  let tab = $state<"prs" | "actions" | "gitlab" | "docker" | "aws" | "inc">("prs");
  // #59 AWS in-pane resource browser.
  let awsSvc = $state<"ec2" | "s3" | "lambda" | "rds" | "ecs" | "eks">("ec2");
  let awsOut = $state("");
  let awsBusy = $state(false);
  async function loadAws() {
    awsBusy = true; awsOut = "Loading…";
    try { awsOut = await invoke<string>("aws_list", { service: awsSvc }); }
    catch (e) { awsOut = String(e); }
    awsBusy = false;
  }
  let prs = $state("");

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
  async function savePromQuery() {
    if (!promQuery.trim()) return;
    const name = await askText({ title: "Save query", value: promQuery.slice(0, 40) });
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

  // Detect expired/invalid cloud creds in the PRs panel output.
  const AUTH_RE = /expired|credentials|unauthorized|not logged in|sso session|reauthenticate|InvalidIdentityToken|token has expired|failed to get token/i;
  const ciAuthErr = $derived(AUTH_RE.test(prs));
  function runCmd(cmd: string) { onRunCommand?.(cmd); }
  // #27 PR review: open a PR's body + comments inline, post a comment.
  let prSel = $state("");
  let prDetail = $state("");
  let prComment = $state("");
  let prBusy = $state(false);
  let prRows = $state<PrRow[]>([]);
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
  async function reviewPr(action: "approve" | "request-changes") {
    if (!prSel || prBusy) return;
    if (action === "request-changes" && !prComment.trim()) { prDetail = "Add a comment before requesting changes.\n\n" + prDetail; return; }
    prBusy = true;
    try { await invoke("gh_pr_review", { cwd, num: prSel, action, body: prComment }); prComment = ""; await openPr(prSel); }
    catch (e) { prDetail = String(e) + "\n\n" + prDetail; } finally { prBusy = false; }
  }
  async function diffPr() {
    if (!prSel) return;
    prDetail = "Loading diff…";
    try { prDetail = await invoke<string>("gh_pr_diff", { cwd, num: prSel }); } catch (e) { prDetail = String(e); }
  }
  // Merge the selected PR (squash/rebase/merge), deleting the branch.
  async function mergePr(method: "squash" | "merge" | "rebase") {
    if (!prSel || prBusy) return;
    if (!confirm(`${method} PR #${prSel} and delete its branch?`)) return;
    prBusy = true;
    try {
      const out = await invoke<string>("gh_pr_merge", { cwd, num: prSel, method });
      toast(`Merged #${prSel} (${method})`, "success");
      prSel = ""; prDetail = ""; await loadPRs();
      if (!out) { /* noop */ }
    } catch (e) { toast(String(e).slice(0, 140) || "merge failed", "error"); }
    finally { prBusy = false; }
  }
  // Open a PR for the current branch (gh pr create --fill).
  async function createPr() {
    if (busy) return;
    busy = true;
    try {
      const out = await invoke<string>("gh_pr_create", { cwd });
      const url = out.trim().split("\n").find((l) => l.startsWith("http")) || "PR created";
      toast(url, "success");
      await loadPRs();
    } catch (e) { toast(String(e).slice(0, 160) || "gh pr create failed", "error"); }
    finally { busy = false; }
  }
  async function loadPRs() {
    busy = true;
    // JSON variant carries CI check status so we can sort failing-first; keep the
    // raw string in `prs` for the auth-error banner + empty-state fallback.
    try {
      prs = await invoke<string>("gh_prs_json", { cwd });
      prRows = parsePrRows(prs);
    } catch (e) {
      prs = String(e);
      prRows = [];
    }
    busy = false;
  }
  // Re-run a failed PR's checks (gh run rerun on the PR's branch), in a terminal.
  function rerunChecks(r: PrRow) {
    if (!r.branch) return;
    runCmd(`gh run list --branch ${r.branch} -L 1 --json databaseId -q '.[0].databaseId' | xargs gh run rerun`);
  }
  // GitHub Actions runs — failing-first, with re-run / log / investigate.
  let runs = $state<RunRow[]>([]);
  let runsErr = $state("");
  let runsBusy = $state(false);
  const runFails = $derived(failingRuns(runs));
  const runsAuthErr = $derived(/not authenticated|gh auth|HTTP 401/i.test(runsErr));
  async function loadActions() {
    runsBusy = true;
    runsErr = "";
    try {
      runs = parseRuns(await invoke<string>("gh_runs_json", { cwd }));
    } catch (e) {
      runsErr = String(e);
      runs = [];
    }
    runsBusy = false;
  }
  async function rerunRun(r: RunRow) {
    try { await invoke("gh_rerun", { cwd, id: r.id }); toast(`Re-running ${r.workflow || r.id}`, "success"); setTimeout(loadActions, 1500); }
    catch (e) { toast(String(e).slice(0, 120), "error"); }
  }
  function runLog(r: RunRow) { runCmd(`gh run view ${r.id} --log${r.state === "fail" ? "-failed" : ""}`); }

  // GitLab CI (#54) — via the authed glab CLI, run in the repo cwd.
  let glabOut = $state("");
  async function loadGlab() {
    busy = true;
    try { glabOut = await invoke<string>("glab_pipelines", { cwd }); }
    catch (e) { glabOut = String(e); }
    busy = false;
  }
  function refresh() {
    if (tab === "prs") loadPRs();
  }

  // Apply AWS profile + GitHub token from Accounts so kubectl / gh use them.
  async function applyCreds() {
    const aws = ACCOUNTS.find((a) => a.key === "aws-profile");
    const gh = ACCOUNTS.find((a) => a.key === "github-token");
    try { if (aws) await invoke("set_aws_profile", { profile: await getValue(aws) }); } catch (e) { console.warn("set_aws_profile failed", e); }
    try { if (gh) await invoke("set_github_token", { token: await getValue(gh) }); } catch (e) { console.warn("set_github_token failed", e); }
  }
  onMount(async () => { await applyCreds(); });
  $effect(() => { if (tab === "prs" && !prs) loadPRs(); });
</script>

<div class="dev">
  <div class="tabs">
    <button class:on={tab === "prs"} onclick={() => (tab = "prs")}><Icon name="pr" size={14} /> Pull Requests</button>
    <button class:on={tab === "actions"} onclick={() => { tab = "actions"; if (!runs.length) loadActions(); }}><Icon name="ci" size={14} /> Actions{#if runFails}<span class="tabbadge">{runFails}</span>{/if}</button>
    <button class:on={tab === "gitlab"} onclick={() => { tab = "gitlab"; if (!glabOut) loadGlab(); }}><Icon name="ci" size={14} /> GitLab CI</button>
    <button class:on={tab === "docker"} onclick={() => (tab = "docker")}><Icon name="docker" size={14} /> Docker</button>
    <button class:on={tab === "aws"} onclick={() => { tab = "aws"; if (!awsOut) loadAws(); }}><Icon name="devops" size={14} /> AWS</button>
    <button class:on={tab === "inc"} onclick={() => (tab = "inc")}><Icon name="alert" size={14} /> Incident</button>
    <span class="sp"></span>
    {#if busy}<span class="busy">…</span>{/if}
    <button class="refresh" onclick={refresh} title="Refresh"><Icon name="refresh" size={13} /></button>
  </div>

  {#if ciAuthErr && tab === "prs"}
    <div class="authbar">
      <span class="aw">⚠ GitHub CLI not authenticated.</span>
      <button onclick={() => runCmd("gh auth login")}>gh auth login</button>
      <button class="ghost" onclick={refresh}>Retry</button>
    </div>
  {/if}

  {#if tab === "prs"}
    <div class="bar"><span class="lbl">Open PRs · {cwd.split("/").pop()}</span>
      <span class="grow"></span>
      <button class="refresh" disabled={busy} onclick={createPr} title="Open a PR for the current branch (gh pr create --fill)">+ New PR</button>
      <button class="refresh" disabled={busy} onclick={loadPRs} title="Refresh"><Icon name="refresh" size={13} /></button>
    </div>
    {#if prRows.length}
      <div class="podlist">
        {#each prRows as r (r.num)}
          <div class="podrow" class:cur={prSel === r.num} role="button" tabindex="0" onclick={() => openPr(r.num)} onkeydown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), openPr(r.num))}>
            <span class="ck ck-{r.checks}" title={r.checks === "none" ? "No checks" : `Checks: ${r.checks}`}></span>
            <span class="bdg" style="color:var(--accent)">#{r.num}</span><span class="pnm">{r.title}{r.draft ? " · draft" : ""}</span>
            {#if r.checks === "fail"}
              {#if onInvestigate}<button class="rerun ai" title="Investigate failing checks with the agent" onclick={(e) => { e.stopPropagation(); onInvestigate(githubInvestigation(r.num, r.branch)); }}><Icon name="agent" size={11} /></button>{/if}
              <button class="rerun" title="Re-run failed checks (in terminal)" onclick={(e) => { e.stopPropagation(); rerunChecks(r); }}><Icon name="refresh" size={11} /></button>
            {/if}
          </div>
        {/each}
      </div>
      {#if prSel}
        <div class="pr-acts">
          <button class="pr-btn" disabled={prBusy} onclick={diffPr} title="Show the PR diff">Diff</button>
          <button class="pr-btn" disabled={prBusy} onclick={() => openPr(prSel)} title="Back to conversation">Conversation</button>
          <span class="grow"></span>
          <button class="pr-btn ok" disabled={prBusy} onclick={() => reviewPr("approve")} title="Approve (optionally with the comment below)">Approve</button>
          <button class="pr-btn warn" disabled={prBusy} onclick={() => reviewPr("request-changes")} title="Request changes (needs a comment)">Request changes</button>
          <button class="pr-btn ok" disabled={prBusy} onclick={() => mergePr("squash")} title="Squash & merge, delete branch">Squash</button>
          <button class="pr-btn" disabled={prBusy} onclick={() => mergePr("merge")} title="Merge commit, delete branch">Merge</button>
        </div>
        <pre class="out">{prDetail}</pre>
        <div class="bar">
          <input class="url" bind:value={prComment} onkeydown={(e) => e.key === "Enter" && postPrComment()} placeholder={`Comment / review body for #${prSel} (Enter to comment)`} spellcheck="false" />
          <button class="refresh" disabled={prBusy || !prComment.trim()} onclick={postPrComment} title="Post comment"><Icon name="play" size={13} /></button>
        </div>
      {/if}
    {:else}
      <pre class="out">{prs.trimStart().startsWith("[") ? "No open PRs." : prs || "No open PRs / gh unavailable."}</pre>
    {/if}
  {:else if tab === "actions"}
    <div class="bar">
      <span class="lbl">GitHub Actions · {cwd.split("/").pop()}</span>
      <span class="grow"></span>
      <button class="refresh" onclick={loadActions} title="Refresh (gh run list)" disabled={runsBusy}><Icon name="refresh" size={13} /></button>
    </div>
    {#if runsErr}
      <pre class="out">{runsAuthErr ? "Not authenticated — run `gh auth login`." : runsErr.slice(0, 200)}</pre>
    {:else if runs.length}
      <div class="podlist">
        {#each runs as r (r.id)}
          <div class="podrow">
            <span class="ck ck-{r.state === 'running' ? 'pending' : r.state}" title={r.state}></span>
            <span class="pnm">{r.workflow ? r.workflow + " · " : ""}{r.title}</span>
            <span class="rbranch" title={r.branch}>{r.branch}</span>
            {#if onInvestigate && r.state === "fail"}<button class="rerun ai" title="Investigate this run with the agent" onclick={() => onInvestigate(actionsInvestigation(r.id, r.workflow, r.branch))}><Icon name="agent" size={11} /></button>{/if}
            <button class="rerun" title="View log in terminal" onclick={() => runLog(r)}><Icon name="terminal" size={11} /></button>
            {#if r.state !== "running"}<button class="rerun" title="Re-run" onclick={() => rerunRun(r)}><Icon name="refresh" size={11} /></button>{/if}
          </div>
        {/each}
      </div>
    {:else}
      <pre class="out">{runsBusy ? "Loading…" : "No recent runs / gh unavailable."}</pre>
    {/if}
  {:else if tab === "gitlab"}
    <div class="bar"><span class="lbl">GitLab CI · {cwd.split("/").pop()}</span>
      <button class="refresh" onclick={loadGlab} title="Refresh (glab ci list)"><Icon name="refresh" size={13} /></button>
      <button class="refresh" title="Trace latest pipeline in terminal" onclick={() => runCmd("glab ci trace")}>trace</button>
      <button class="refresh" title="Retry pipeline in terminal" onclick={() => runCmd("glab ci retry")}>retry</button>
    </div>
    <pre class="out">{glabOut || "Loading… (needs glab + a GitLab remote)"}</pre>
  {:else if tab === "docker"}
    <Docker {cwd} {onRunCommand} />
  {:else if tab === "aws"}
    <div class="bar">
      <span class="lbl">AWS</span>
      <select class="sel" bind:value={awsSvc} onchange={loadAws}><option value="ec2">EC2</option><option value="s3">S3</option><option value="lambda">Lambda</option><option value="rds">RDS</option><option value="ecs">ECS</option><option value="eks">EKS</option></select>
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
  .pr-acts { display: flex; align-items: center; gap: 6px; padding: 6px 12px; border-bottom: 1px solid var(--hairline); }
  .pr-acts .grow { flex: 1; }
  .pr-btn { border: 1px solid var(--border); background: var(--panel2); color: var(--text2); font-family: var(--font-ui);
    font-size: 11px; padding: 3px 9px; border-radius: 5px; cursor: default; }
  .pr-btn:hover:not(:disabled) { color: var(--text); border-color: var(--text3); }
  .pr-btn.ok:hover:not(:disabled) { color: var(--green); border-color: var(--green); }
  .pr-btn.warn:hover:not(:disabled) { color: var(--accent2); border-color: var(--accent2); }
  .pr-btn:disabled { opacity: 0.45; }
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
  .sparks { display: flex; flex-direction: column; background: var(--panel); }
  .spark { display: flex; flex-direction: column; background: transparent; padding: 4px 12px 3px;
    border-bottom: 1px solid var(--border); cursor: default; }
  .spark:hover { background: color-mix(in srgb, var(--text) 6%, transparent); }
  .spark-top { display: flex; align-items: baseline; gap: 6px; min-height: 22px; }
  .spark-nm { flex: 1; font-size: 10.5px; color: var(--text3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .spark-val { font-family: var(--font-mono); font-size: 12px; color: var(--text); }
  .spark-svg { display: block; width: 100%; height: 18px; }
  .sevdot { flex: 0 0 auto; width: 8px; height: 8px; border-radius: 50%; background: var(--text3); margin-right: 8px; }
  .sevdot.crit { background: var(--danger, #e5484d); }
  .sevdot.warn { background: var(--warn, #f5a623); }
  .alabels { color: var(--text3); font-size: 11px; }
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
  .pnm { flex: 1; min-width: 0; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pst { flex: 0 0 auto; width: 92px; text-align: right; }
  .tabbadge { margin-left: 5px; padding: 0 5px; border-radius: 8px; font-size: 9.5px; font-weight: 600;
    color: #fff; background: var(--status-failure); }
  .rbranch { flex: 0 0 auto; max-width: 160px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    color: var(--text3); font-size: 10.5px; }
  .ck { flex: 0 0 auto; width: 8px; height: 8px; border-radius: 50%; background: var(--text3); }
  .ck-fail { background: var(--status-failure); }
  .ck-pending { background: var(--status-attention); }
  .ck-pass { background: var(--status-verified); }
  .ck-none { background: var(--text3); opacity: 0.4; }
  .rerun { flex: 0 0 auto; display: inline-flex; align-items: center; justify-content: center;
    width: 20px; height: 18px; border: 1px solid color-mix(in srgb, var(--red) 40%, transparent);
    border-radius: 5px; background: transparent; color: var(--red); cursor: default; }
  .rerun:hover { background: color-mix(in srgb, var(--red) 12%, transparent); }
  .rerun.ai { border-color: color-mix(in srgb, var(--status-agent) 45%, transparent); color: var(--status-agent); }
  .rerun.ai:hover { background: color-mix(in srgb, var(--status-agent) 12%, transparent); }
</style>
