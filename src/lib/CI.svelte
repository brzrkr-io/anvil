<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import Skeleton from "$lib/Skeleton.svelte";
  import EmptyState from "$lib/EmptyState.svelte";
  import { toast } from "$lib/toast";
  import { askConfirm } from "$lib/dialog";
  import { readCache, writeCache } from "$lib/cache";
  import { gitlabInvestigation } from "$lib/agent-ops";

  let { cwd, onRunCommand, onInvestigate, active = true }: { cwd: string; onRunCommand?: (cmd: string) => void; onInvestigate?: (prompt: string) => void; active?: boolean } = $props();

  interface Pipeline {
    id: number;
    iid: number;
    status: string;
    ref: string;
    sha: string;
    source: string;
    web_url: string;
    created_at: string;
    updated_at: string;
  }

  interface Job {
    id: number;
    name: string;
    stage: string;
    status: string;
    started_at: string | null;
    finished_at: string | null;
    duration: number | null;
    web_url: string;
  }

  let pipelines = $state<Pipeline[]>([]);
  let selectedPipeline = $state<Pipeline | null>(null);
  let jobs = $state<Job[]>([]);
  let selectedJob = $state<Job | null>(null);
  let logContent = $state("");
  let ciErr = $state("");
  let busy = $state(false);
  let lastUpdated = $state(0);

  let pollInterval: ReturnType<typeof setInterval> | null = null;
  let jobPollInterval: ReturnType<typeof setInterval> | null = null;
  let logPollInterval: ReturnType<typeof setInterval> | null = null;
  let logEl = $state<HTMLDivElement | null>(null);
  let logWrap = $state(false);
  // #25 Pipeline DAG: lay stages out as left→right columns with edges between
  // them (GitLab's default stage-sequential graph), vs. the flat list.
  let dagView = $state(true);

  const RUNNING_STATUSES = new Set(["running", "pending", "created", "waiting_for_resource", "preparing", "scheduled"]);

  function isRunning(status: string) {
    return RUNNING_STATUSES.has(status);
  }

  function statusColor(status: string): string {
    if (status === "success") return "var(--green)";
    if (isRunning(status)) return "var(--accent)";
    if (status === "failed") return "var(--red)";
    if (status === "canceled" || status === "skipped") return "var(--text3)";
    return "var(--yellow)";
  }

  function relativeAge(iso: string): string {
    const diff = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
    if (diff < 60) return `${diff}s`;
    if (diff < 3600) return `${Math.floor(diff / 60)}m`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
    return `${Math.floor(diff / 86400)}d`;
  }

  function fmtDuration(secs: number | null | undefined): string {
    if (!secs && secs !== 0) return "—";
    const s = Math.round(secs);
    if (s < 60) return `${s}s`;
    return `${Math.floor(s / 60)}m${s % 60}s`;
  }

  function shortSha(sha: string): string {
    return sha.slice(0, 8);
  }

  // GitLab pipeline sources are verbose (merge_request_event, …). Show a short
  // readable label so the column never truncates.
  function srcLabel(src: string): string {
    const map: Record<string, string> = {
      merge_request_event: "merge request",
      external_pull_request_event: "external PR",
      push: "push",
      web: "web",
      schedule: "schedule",
      trigger: "trigger",
      pipeline: "pipeline",
      api: "api",
      chat: "chat",
      parent_pipeline: "parent",
    };
    return map[src] ?? src.replace(/_event$/, "").replace(/_/g, " ");
  }

  // GitLab job traces are full of ANSI color codes + section markers — strip to
  // readable plain text.
  function cleanTrace(s: string): string {
    return s
      .replace(/\x1b\][^\x1b\x07]*(?:\x07|\x1b\\)/g, "")
      .replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "")
      .replace(/section_(?:start|end):\d+:[\w.-]+/g, "")
      .replace(/\r(?=[^\n])/g, "")
      .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f]/g, "");
  }

  const jobsDone = $derived(jobs.filter((j) => !isRunning(j.status)).length);

  // GitLab "timestamps" trace prefixes each line with an RFC3339 stamp + a short
  // stream code. Split it off into a dim gutter and classify the body by level.
  const TS_RE = /^(\d{4}-\d\d-\d\dT[\d:.]+Z?)\s+(?:[0-9a-fA-F]{2,4}\s+)?/;
  interface LogLine { ts: string; text: string; cls: string; }
  const logLines = $derived.by<LogLine[]>(() => {
    if (!logContent || logContent === "Loading…") return [];
    const lines = logContent.replace(/\n+$/, "").split("\n");
    const out: LogLine[] = [];
    for (let line of lines) {
      let ts = "";
      const m = line.match(TS_RE);
      if (m) { ts = m[1].slice(11, 19); line = line.slice(m[0].length); }
      let cls = "";
      if (/(^|\b)(ERROR|FATAL|error:|errors? occurred|failed|✗|✖)\b/i.test(line)) cls = "err";
      else if (/(^|\b)(WARN(?:ING)?|skipp(?:ed|ing))\b/i.test(line)) cls = "warn";
      else if (/(completed successfully|✓|✅|succeeded|passed|201 Created)\b/i.test(line)) cls = "ok";
      else if (/(^|\b)INFO\b/.test(line)) cls = "info";
      else if (/^\s*[$#] /.test(line)) cls = "cmd";
      out.push({ ts, text: line, cls });
    }
    return out;
  });

  const hasErr = $derived(logLines.some((l) => l.cls === "err"));

  // Scroll the first ERROR line into view (#23). Returns false if none found.
  function scrollToError(): boolean {
    const el = logEl?.querySelector<HTMLElement>(".lg.err");
    if (!el) return false;
    el.scrollIntoView({ block: "center" });
    return true;
  }

  const stageGroups = $derived.by<Map<string, Job[]>>(() => {
    const m = new Map<string, Job[]>();
    for (const j of jobs) {
      const arr = m.get(j.stage) ?? [];
      arr.push(j);
      m.set(j.stage, arr);
    }
    return m;
  });

  const stageList = $derived([...stageGroups.entries()]);

  async function loadPipelines() {
    if (document.hidden || !active) return;
    try {
      const raw = await invoke<string>("glab_pipelines_json", { cwd });
      pipelines = JSON.parse(raw) as Pipeline[];
      writeCache(`ci-pipelines:${cwd}`, pipelines);
      ciErr = "";
      lastUpdated = Date.now();
      if (selectedPipeline) {
        const updated = pipelines.find((p) => p.id === selectedPipeline!.id);
        if (updated) selectedPipeline = updated;
      }
    } catch (e) {
      ciErr = String(e);
    }
  }

  async function loadJobs(pipelineId: number) {
    if (document.hidden || !active) return;
    try {
      const raw = await invoke<string>("glab_pipeline_jobs", { cwd, pipeline: String(pipelineId) });
      // glab returns jobs newest-first; ascending id = pipeline stage order (validate→build→deploy).
      jobs = (JSON.parse(raw) as Job[]).sort((a, b) => a.id - b.id);
    } catch (e) {
      jobs = [];
    }
  }

  async function loadTrace(jobId: number) {
    if (document.hidden || !active) return;
    try {
      const raw = await invoke<string>("glab_job_trace", { cwd, job: String(jobId) });
      logContent = cleanTrace(raw);
      // Jump to the first error on a failed trace; otherwise tail to bottom (#23).
      requestAnimationFrame(() => {
        if (!scrollToError() && logEl) logEl.scrollTop = logEl.scrollHeight;
      });
    } catch (e) {
      logContent = String(e);
    }
  }

  function startPipelinePoll() {
    if (pollInterval) clearInterval(pollInterval);
    pollInterval = setInterval(loadPipelines, 5000);
  }

  function startJobPoll(pipelineId: number) {
    if (jobPollInterval) clearInterval(jobPollInterval);
    if (selectedPipeline && isRunning(selectedPipeline.status)) {
      jobPollInterval = setInterval(() => loadJobs(pipelineId), 5000);
    }
  }

  function startLogPoll(jobId: number, jobStatus: string) {
    if (logPollInterval) clearInterval(logPollInterval);
    if (isRunning(jobStatus)) {
      logPollInterval = setInterval(() => loadTrace(jobId), 4000);
    }
  }

  async function selectPipeline(p: Pipeline) {
    selectedPipeline = p;
    selectedJob = null;
    logContent = "";
    if (logPollInterval) { clearInterval(logPollInterval); logPollInterval = null; }
    jobs = [];
    await loadJobs(p.id);
    startJobPoll(p.id);
  }

  async function selectJob(j: Job) {
    selectedJob = j;
    logContent = "Loading…";
    if (logPollInterval) clearInterval(logPollInterval);
    await loadTrace(j.id);
    startLogPoll(j.id, j.status);
  }

  function closeLog() {
    selectedJob = null;
    logContent = "";
    if (logPollInterval) { clearInterval(logPollInterval); logPollInterval = null; }
  }

  async function retryPipeline(p: Pipeline) {
    try {
      await invoke("glab_pipeline_retry", { cwd, pipeline: String(p.id) });
      toast(`Pipeline #${p.iid} queued for retry`, "success");
      await loadPipelines();
    } catch (e) {
      toast(String(e).slice(0, 120), "error");
    }
  }

  async function cancelPipeline(p: Pipeline) {
    const ok = await askConfirm({
      title: "Cancel pipeline",
      message: `Cancel pipeline #${p.iid} (${p.ref})?`,
      danger: true,
    });
    if (!ok) return;
    try {
      await invoke("glab_pipeline_cancel", { cwd, pipeline: String(p.id) });
      toast(`Pipeline #${p.iid} canceled`, "success");
      await loadPipelines();
    } catch (e) {
      toast(String(e).slice(0, 120), "error");
    }
  }

  function openInGitLab(p: Pipeline) {
    onRunCommand?.(`open "${p.web_url}"`);
  }

  async function retryJob(j: Job) {
    try {
      await invoke("glab_job_retry", { cwd, job: String(j.id) });
      toast(`Retrying ${j.name}`, "success");
      if (selectedPipeline) await loadJobs(selectedPipeline.id);
    } catch (e) {
      toast(String(e).slice(0, 120), "error");
    }
  }

  async function playJob(j: Job) {
    try {
      await invoke("glab_job_play", { cwd, job: String(j.id) });
      toast(`Started ${j.name}`, "success");
      if (selectedPipeline) await loadJobs(selectedPipeline.id);
    } catch (e) {
      toast(String(e).slice(0, 120), "error");
    }
  }

  let secondsAgo = $state(0);
  let agoInterval: ReturnType<typeof setInterval> | null = null;

  function updateAgo() {
    secondsAgo = lastUpdated ? Math.floor((Date.now() - lastUpdated) / 1000) : 0;
  }

  onMount(async () => {
    // Show last-known pipelines instantly from cache, then refresh.
    pipelines = readCache<Pipeline[]>(`ci-pipelines:${cwd}`) ?? pipelines;
    busy = pipelines.length === 0;
    await loadPipelines();
    busy = false;
    startPipelinePoll();
    agoInterval = setInterval(updateAgo, 1000);
  });

  onDestroy(() => {
    if (pollInterval) clearInterval(pollInterval);
    if (jobPollInterval) clearInterval(jobPollInterval);
    if (logPollInterval) clearInterval(logPollInterval);
    if (agoInterval) clearInterval(agoInterval);
  });
</script>

<div class="ci">
  <!-- Header bar -->
  <div class="topbar">
    <span class="lbl">GitLab CI</span>
    <span class="spacer"></span>
    {#if lastUpdated}
      <span class="ago">{secondsAgo}s ago</span>
    {/if}
    {#if busy}<span class="spin">…</span>{/if}
    <button class="iconbtn" onclick={async () => { busy = true; await loadPipelines(); if (selectedPipeline) await loadJobs(selectedPipeline.id); busy = false; }} title="Refresh">
      <Icon name="refresh" size={13} />
    </button>
  </div>

  <!-- Error state -->
  {#if ciErr}
    <div class="ci-err">
      <span class="err-text">{ciErr.includes("glab not found") ? "glab not found in PATH." : ciErr.includes("auth") || ciErr.includes("401") ? "Not authenticated." : ciErr.slice(0, 200)}</span>
      {#if ciErr.includes("auth") || ciErr.includes("401") || ciErr.includes("GITLAB_TOKEN")}
        <span class="err-hint">Run <code>glab auth login</code> to authenticate.</span>
      {/if}
    </div>
  {/if}

  <!-- Body: pipeline list + jobs + log -->
  <div class="body">
    <!-- Pipeline list -->
    <div class="pipelines" class:has-jobs={!!selectedPipeline}>
      {#if !ciErr && pipelines.length === 0 && busy}
        <Skeleton rows={12} />
      {:else if !ciErr && pipelines.length === 0 && !busy}
        <EmptyState icon="ci" title="No pipelines found" hint="No recent GitLab pipelines for this project." />
      {:else if !ciErr}
        <div class="pl-header">
          <span class="col-dot"></span>
          <span class="col-iid">#</span>
          <span class="col-ref">Ref</span>
          <span class="col-src">Source</span>
          <span class="col-sha">SHA</span>
          <span class="col-age">Age</span>
          <span class="col-dur">Duration</span>
          <span class="col-acts"></span>
        </div>
        {#each pipelines as p (p.id)}
          {@const dur = p.status === "success" || p.status === "failed" || p.status === "canceled"
            ? (p.updated_at && p.created_at
                ? Math.round((new Date(p.updated_at).getTime() - new Date(p.created_at).getTime()) / 1000)
                : null)
            : null}
          <div
            class="pl-row"
            class:selected={selectedPipeline?.id === p.id}
            role="button"
            tabindex="0"
            onclick={() => selectPipeline(p)}
            onkeydown={(e) => e.key === "Enter" && selectPipeline(p)}
          >
            <span class="col-dot">
              <span class="dot" class:running={isRunning(p.status)} style="background:{statusColor(p.status)}"></span>
            </span>
            <span class="col-iid muted">#{p.iid}</span>
            <span class="col-ref mono">{p.ref}</span>
            <span class="col-src muted" title={p.source}>{srcLabel(p.source)}</span>
            <span class="col-sha mono muted">{shortSha(p.sha)}</span>
            <span class="col-age muted">{relativeAge(p.updated_at)}</span>
            <span class="col-dur muted">{fmtDuration(dur)}</span>
            <span class="col-acts">
              {#if isRunning(p.status)}
                <button class="act warn" title="Cancel" onclick={(e) => { e.stopPropagation(); cancelPipeline(p); }}>
                  <Icon name="close" size={11} />
                </button>
              {:else}
                <button class="act" title="Retry" onclick={(e) => { e.stopPropagation(); retryPipeline(p); }}>
                  <Icon name="refresh" size={11} />
                </button>
              {/if}
              {#if onInvestigate && p.status === "failed"}
                <button class="act ai" title="Investigate this pipeline with the agent" onclick={(e) => { e.stopPropagation(); onInvestigate(gitlabInvestigation(String(p.id), p.ref)); }}>
                  <Icon name="agent" size={11} />
                </button>
              {/if}
              <button class="act" title="Open in GitLab" onclick={(e) => { e.stopPropagation(); openInGitLab(p); }}>
                <Icon name="zoom" size={11} />
              </button>
            </span>
          </div>
        {/each}
      {/if}
    </div>

    <!-- Jobs panel (right side when pipeline selected) -->
    {#if selectedPipeline}
      <div class="jobs-panel" class:has-log={!!selectedJob}>
        <div class="jobs-head">
          <span class="jobs-title">Pipeline #{selectedPipeline.iid}</span>
          <span class="jobs-status" style="color:{statusColor(selectedPipeline.status)}">{selectedPipeline.status}</span>
          {#if jobs.length}<span class="jobs-prog">{jobsDone}/{jobs.length}</span>{/if}
          <span class="spacer"></span>
          {#if jobs.length}<button class="iconbtn" class:on={dagView} onclick={() => (dagView = !dagView)} title="{dagView ? 'List view' : 'DAG view'}">{dagView ? '☰' : '⛓'}</button>{/if}
          <button class="iconbtn" onclick={() => { selectedPipeline = null; jobs = []; closeLog(); }} title="Close">
            <Icon name="close" size={13} />
          </button>
        </div>
        <div class="jobs-body">
          {#if jobs.length === 0}
            <div class="empty">No jobs.</div>
          {:else if dagView}
            <div class="dag">
              {#each stageList as [stage, stageJobs], si (stage)}
                {#if si > 0}<div class="dag-link" aria-hidden="true"></div>{/if}
                <div class="dag-stage">
                  <div class="dag-stage-head"><span>{stage}</span><span class="stage-count">{stageJobs.length}</span></div>
                  {#each stageJobs as j (j.id)}
                    <div
                      class="dag-node"
                      class:selected={selectedJob?.id === j.id}
                      role="button"
                      tabindex="0"
                      onclick={() => selectJob(j)}
                      onkeydown={(e) => e.key === "Enter" && selectJob(j)}
                      style="border-left-color:{statusColor(j.status)}"
                    >
                      <span class="dot" class:running={isRunning(j.status)} style="background:{statusColor(j.status)}"></span>
                      <span class="job-name" title={j.name}>{j.name}</span>
                      {#if j.status === "manual"}
                        <button class="jact" title="Play job" onclick={(e) => { e.stopPropagation(); playJob(j); }}><Icon name="play" size={10} /></button>
                      {:else if !isRunning(j.status)}
                        <button class="jact" title="Retry job" onclick={(e) => { e.stopPropagation(); retryJob(j); }}><Icon name="refresh" size={10} /></button>
                      {/if}
                    </div>
                  {/each}
                </div>
              {/each}
            </div>
          {:else}
            {#each stageList as [stage, stageJobs] (stage)}
              <div class="stage-label"><span>{stage}</span><span class="stage-count">{stageJobs.length}</span></div>
              {#each stageJobs as j (j.id)}
                <div
                  class="job-row"
                  class:selected={selectedJob?.id === j.id}
                  role="button"
                  tabindex="0"
                  onclick={() => selectJob(j)}
                  onkeydown={(e) => e.key === "Enter" && selectJob(j)}
                >
                  <span class="dot" class:running={isRunning(j.status)} style="background:{statusColor(j.status)}"></span>
                  <span class="job-name">{j.name}</span>
                  <span class="job-dur muted">{fmtDuration(j.duration)}</span>
                  <span class="job-acts">
                    {#if j.status === "manual"}
                      <button class="jact" title="Play job" onclick={(e) => { e.stopPropagation(); playJob(j); }}>
                        <Icon name="play" size={10} />
                      </button>
                    {:else if !isRunning(j.status)}
                      <button class="jact" title="Retry job" onclick={(e) => { e.stopPropagation(); retryJob(j); }}>
                        <Icon name="refresh" size={10} />
                      </button>
                    {/if}
                  </span>
                </div>
              {/each}
            {/each}
          {/if}
        </div>
      </div>
    {/if}

    <!-- Log panel -->
    {#if selectedJob}
      <div class="log-panel">
        <div class="log-head">
          <span class="log-title">{selectedJob.name}</span>
          <span class="log-status" style="color:{statusColor(selectedJob.status)}">{selectedJob.status}</span>
          <span class="log-lines">{logLines.length} lines</span>
          <span class="spacer"></span>
          {#if hasErr}<button class="iconbtn err-jump" onclick={scrollToError} title="Jump to first error">↡ error</button>{/if}
          <button class="iconbtn" class:on={logWrap} onclick={() => (logWrap = !logWrap)} title="Toggle wrap">⤶</button>
          <button class="iconbtn" onclick={closeLog} title="Close">
            <Icon name="close" size={13} />
          </button>
        </div>
        {#if logContent === "Loading…"}
          <div class="log-out empty">Loading…</div>
        {:else}
          <div class="log-out" class:wrap={logWrap} bind:this={logEl}>
            {#each logLines as l}
              <div class="lg {l.cls}">{#if l.ts}<span class="lg-ts">{l.ts}</span>{/if}<span class="lg-tx">{l.text || " "}</span></div>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .ci { display: flex; flex-direction: column; height: 100%; min-height: 0; }

  .topbar {
    display: flex; align-items: center; gap: 8px; height: 28px; flex: 0 0 auto;
    padding: 0 12px; border-bottom: 1px solid var(--border);
  }
  .lbl { color: var(--text3); font-size: 11px; font-weight: 500; }
  .spacer { flex: 1; }
  .spin { color: var(--accent); font-size: 12px; }
  .ago { color: var(--text3); font-size: 10.5px; font-family: var(--font-mono); }

  .iconbtn {
    display: inline-flex; align-items: center; justify-content: center;
    width: 22px; height: 20px; border: 0; border-radius: 5px;
    background: transparent; color: var(--text3); cursor: default;
  }
  .iconbtn:hover { background: var(--sel); color: var(--text); }
  .iconbtn.err-jump { width: auto; padding: 0 7px; gap: 4px; font-size: 11px; color: var(--red); }
  .iconbtn.err-jump:hover { background: color-mix(in srgb, var(--red) 16%, transparent); color: var(--red); }

  .ci-err {
    display: flex; flex-direction: column; gap: 4px;
    padding: 10px 12px; font-size: 11.5px; color: var(--text3);
    border-bottom: 1px solid var(--border); flex: 0 0 auto;
  }
  .err-text { color: var(--text3); }
  .err-hint { font-size: 11px; }
  .err-hint code { font-family: var(--font-mono); color: var(--accent); }

  .empty { padding: 20px 14px; color: var(--text3); font-size: 12px; }

  /* Body layout: pipeline list left, jobs center, log right */
  .body { flex: 1; min-height: 0; display: flex; overflow: hidden; }

  /* Pipeline list */
  .pipelines { flex: 1; min-width: 0; overflow-y: auto; }
  .pipelines.has-jobs { flex: 0 0 45%; border-right: 1px solid var(--border); }

  .pl-header, .pl-row {
    display: grid;
    grid-template-columns: 18px 56px minmax(0,1fr) 132px 96px 56px 80px 56px;
    align-items: center; column-gap: 8px; height: 22px; padding: 0 10px;
    border-bottom: 1px solid var(--hairline); position: relative;
  }
  .pl-header {
    font-size: 10.5px; color: var(--text3); font-weight: 500;
    position: sticky; top: 0; background: var(--panel); z-index: 1;
  }
  .pl-row { font-size: 11.5px; cursor: default; }
  .pl-row:hover { background: color-mix(in srgb, var(--text) 6%, transparent); }
  .pl-row.selected { background: var(--sel); }
  .pl-row:hover .col-acts, .pl-row.selected .col-acts { opacity: 1; }

  .col-dot { display: flex; align-items: center; }
  .dot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; }
  .dot.running { animation: pulse 1.2s ease-in-out infinite; }
  @keyframes pulse {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 55%, transparent); }
    50% { box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 0%, transparent); }
  }
  .col-iid { font-family: var(--font-mono); }
  .col-ref { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
  .col-src { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .col-sha { text-align: right; }
  .col-age { text-align: right; }
  .col-dur { text-align: right; }
  .col-acts {
    position: absolute; right: 6px; top: 0; height: 100%;
    display: flex; align-items: center; gap: 3px;
    padding-left: 12px; background: linear-gradient(to right, transparent, var(--panel) 14px);
    opacity: 0; transition: opacity 0.1s;
  }
  .pl-row:hover .col-acts {
    background: linear-gradient(to right, transparent, color-mix(in srgb, var(--text) 6%, var(--panel)) 14px);
  }
  .pl-row.selected .col-acts {
    background: linear-gradient(to right, transparent, var(--sel) 14px);
  }

  .mono { font-family: var(--font-mono); }
  .muted { color: var(--text3); }

  .act {
    display: inline-flex; align-items: center; justify-content: center;
    width: 18px; height: 16px; border: 1px solid var(--border);
    background: var(--panel2); color: var(--text2); border-radius: 4px; cursor: default;
  }
  .act:hover { color: var(--text); border-color: var(--text3); }
  .act.warn:hover { color: var(--red); border-color: var(--red); }

  /* Jobs panel */
  .jobs-panel {
    flex: 0 0 30%; min-width: 180px; display: flex; flex-direction: column;
    border-right: 1px solid var(--border);
  }
  .jobs-panel.has-log { flex: 0 0 22%; }
  .jobs-head {
    display: flex; align-items: center; gap: 8px; height: 28px; flex: 0 0 auto;
    padding: 0 10px; border-bottom: 1px solid var(--border); font-size: 11.5px;
  }
  .jobs-title { color: var(--text3); font-size: 11px; font-weight: 500; flex: 0 0 auto; }
  .jobs-status { font-size: 11px; flex: 0 0 auto; }
  .jobs-prog { font-family: var(--font-mono); font-size: 10px; color: var(--text3); opacity: 0.7; flex: 0 0 auto; }
  .jobs-body { flex: 1; overflow-y: auto; }

  .stage-label {
    display: flex; align-items: center; justify-content: space-between;
    padding: 5px 10px 3px; font-size: 10px; font-weight: 500;
    color: var(--text3); text-transform: uppercase; letter-spacing: 0.05em;
    border-bottom: 1px solid var(--hairline); background: var(--panel);
    position: sticky; top: 0; z-index: 1;
  }
  .stage-count {
    font-family: var(--font-mono); font-size: 9.5px; letter-spacing: 0;
    color: var(--text3); opacity: 0.7;
  }
  .job-row {
    display: flex; align-items: center; gap: 8px; height: 22px; padding: 0 10px;
    font-size: 11.5px; cursor: default; border-bottom: 1px solid var(--hairline);
  }
  .job-row:hover { background: color-mix(in srgb, var(--text) 6%, transparent); }
  .job-row.selected { background: var(--sel); }
  .job-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
  .job-dur { font-family: var(--font-mono); font-size: 10.5px; }
  .job-acts { display: flex; align-items: center; flex: 0 0 auto; width: 18px; justify-content: flex-end; }
  .jact {
    display: none; align-items: center; justify-content: center;
    width: 16px; height: 15px; border: 1px solid var(--border);
    background: var(--panel2); color: var(--text2); border-radius: 4px; cursor: default;
  }
  .job-row:hover .jact { display: inline-flex; }
  .jact:hover { color: var(--text); border-color: var(--text3); }

  /* #25 Pipeline DAG — stages as left→right columns with edges between them. */
  .dag {
    display: flex; align-items: flex-start; gap: 0;
    padding: 14px 12px; overflow-x: auto; min-height: 100%;
  }
  .dag-stage { display: flex; flex-direction: column; gap: 8px; min-width: 150px; flex: 0 0 auto; }
  .dag-stage-head {
    display: flex; align-items: center; justify-content: space-between; gap: 6px;
    font-size: 10px; font-weight: 500; color: var(--text3);
    text-transform: uppercase; letter-spacing: 0.05em; padding: 0 2px 2px;
  }
  .dag-link {
    flex: 0 0 28px; align-self: center; height: 1px; margin-top: 18px;
    background: var(--border); position: relative;
  }
  .dag-link::after {
    content: ""; position: absolute; right: 0; top: -3px;
    border-left: 5px solid var(--border);
    border-top: 3.5px solid transparent; border-bottom: 3.5px solid transparent;
  }
  .dag-node {
    display: flex; align-items: center; gap: 7px; padding: 0 8px; height: 28px;
    font-size: 11.5px; cursor: default; border: 1px solid var(--border);
    border-left: 3px solid var(--border); border-radius: 6px; background: var(--panel2);
  }
  .dag-node:hover { background: color-mix(in srgb, var(--text) 6%, transparent); }
  .dag-node.selected { background: var(--sel); border-color: var(--text3); }
  .dag-node .jact { display: none; }
  .dag-node:hover .jact { display: inline-flex; }

  /* Log panel */
  .log-panel { flex: 1; min-width: 0; display: flex; flex-direction: column; min-height: 0; }
  .log-head {
    display: flex; align-items: center; gap: 8px; height: 28px; flex: 0 0 auto;
    padding: 0 10px; border-bottom: 1px solid var(--border); font-size: 11.5px;
  }
  .log-title { color: var(--text3); font-size: 11px; font-weight: 500; flex: 0 0 auto; }
  .log-status { font-size: 11px; flex: 0 0 auto; }
  .log-lines { font-family: var(--font-mono); font-size: 10px; color: var(--text3); opacity: 0.6; flex: 0 0 auto; }
  .iconbtn.on { background: var(--sel); color: var(--text); }

  .log-out {
    flex: 1; min-height: 0; overflow: auto; margin: 0; padding: 6px 0;
    font-family: var(--font-mono); font-size: 11px; line-height: 1.55;
    color: var(--text2); background: var(--bg);
  }
  .log-out.empty { padding: 16px 12px; color: var(--text3); font-family: var(--font-ui); font-size: 12px; }

  .lg {
    display: flex; gap: 10px; padding: 0 12px; white-space: pre; min-width: max-content;
  }
  .log-out.wrap .lg { white-space: pre-wrap; min-width: 0; word-break: break-word; }
  .lg:hover { background: color-mix(in srgb, var(--text) 4%, transparent); }
  .lg-ts {
    flex: 0 0 auto; color: var(--text3); opacity: 0.5; user-select: none;
    -webkit-user-select: none;
  }
  .lg-tx { flex: 1 1 auto; min-width: 0; }
  .lg.err .lg-tx { color: var(--red); }
  .lg.warn .lg-tx { color: var(--yellow); }
  .lg.ok .lg-tx { color: var(--green); }
  .lg.info .lg-tx { color: var(--text2); }
  .lg.cmd .lg-tx { color: var(--accent); font-weight: 600; }
</style>
