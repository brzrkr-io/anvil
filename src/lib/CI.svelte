<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import { toast } from "$lib/toast";
  import { askConfirm } from "$lib/dialog";

  let { cwd, onRunCommand }: { cwd: string; onRunCommand?: (cmd: string) => void } = $props();

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
  let logEl = $state<HTMLPreElement | null>(null);

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

  const stageGroups = $derived.by<Map<string, Job[]>>(() => {
    const m = new Map<string, Job[]>();
    for (const j of jobs) {
      const arr = m.get(j.stage) ?? [];
      arr.push(j);
      m.set(j.stage, arr);
    }
    return m;
  });

  async function loadPipelines() {
    if (document.hidden) return;
    try {
      const raw = await invoke<string>("glab_pipelines_json", { cwd });
      pipelines = JSON.parse(raw) as Pipeline[];
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
    if (document.hidden) return;
    try {
      const raw = await invoke<string>("glab_pipeline_jobs", { cwd, pipeline: String(pipelineId) });
      // glab returns jobs newest-first; ascending id = pipeline stage order (validate→build→deploy).
      jobs = (JSON.parse(raw) as Job[]).sort((a, b) => a.id - b.id);
    } catch (e) {
      jobs = [];
    }
  }

  async function loadTrace(jobId: number) {
    if (document.hidden) return;
    try {
      const raw = await invoke<string>("glab_job_trace", { cwd, job: String(jobId) });
      logContent = raw;
      if (logEl) {
        logEl.scrollTop = logEl.scrollHeight;
      }
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

  let secondsAgo = $state(0);
  let agoInterval: ReturnType<typeof setInterval> | null = null;

  function updateAgo() {
    secondsAgo = lastUpdated ? Math.floor((Date.now() - lastUpdated) / 1000) : 0;
  }

  onMount(async () => {
    busy = true;
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
      {#if !ciErr && pipelines.length === 0 && !busy}
        <div class="empty">No pipelines found.</div>
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
            <span class="col-src muted">{p.source}</span>
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
          <span class="spacer"></span>
          <button class="iconbtn" onclick={() => { selectedPipeline = null; jobs = []; closeLog(); }} title="Close">
            <Icon name="close" size={13} />
          </button>
        </div>
        <div class="jobs-body">
          {#if jobs.length === 0}
            <div class="empty">No jobs.</div>
          {:else}
            {#each [...stageGroups.entries()] as [stage, stageJobs] (stage)}
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
          <span class="spacer"></span>
          <button class="iconbtn" onclick={closeLog} title="Close">
            <Icon name="close" size={13} />
          </button>
        </div>
        <pre class="log-out" bind:this={logEl}>{logContent}</pre>
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
    grid-template-columns: 16px 44px minmax(0,1fr) 72px 72px 44px 56px 56px;
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

  /* Log panel */
  .log-panel { flex: 1; min-width: 0; display: flex; flex-direction: column; min-height: 0; }
  .log-head {
    display: flex; align-items: center; gap: 8px; height: 28px; flex: 0 0 auto;
    padding: 0 10px; border-bottom: 1px solid var(--border); font-size: 11.5px;
  }
  .log-title { color: var(--text3); font-size: 11px; font-weight: 500; flex: 0 0 auto; }
  .log-status { font-size: 11px; flex: 0 0 auto; }
  .log-out {
    flex: 1; min-height: 0; overflow: auto; margin: 0; padding: 10px 12px;
    font-family: var(--font-mono); font-size: 11px; line-height: 1.5;
    color: var(--text2); white-space: pre; background: var(--bg);
  }
</style>
