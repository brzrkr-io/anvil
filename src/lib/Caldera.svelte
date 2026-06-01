<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  interface Run { agent: string; step: string; status: string; summary: string; }
  interface Snapshot { online: boolean; project: string; branch: string; runs: Run[]; attention: string[]; }

  let snap = $state<Snapshot>({ online: false, project: "", branch: "", runs: [], attention: [] });
  let timer: ReturnType<typeof setInterval> | undefined;

  async function poll() {
    try { snap = await invoke<Snapshot>("caldera_snapshot"); } catch { /* keep last */ }
  }
  onMount(() => { poll(); timer = setInterval(poll, 4000); });
  onDestroy(() => { if (timer) clearInterval(timer); });

  const statusColor = (s: string) =>
    s === "passed" || s === "ok" ? "var(--green)"
      : /fail|error|blocked/.test(s) ? "var(--red)"
      : s === "open" || s === "running" ? "var(--yellow)"
      : "var(--text3)";
</script>

<div class="cal">
  <div class="head">
    <span class="dot" style="background:{snap.online ? 'var(--green)' : 'var(--text3)'}"></span>
    <span class="ttl">Caldera</span>
    <span class="muted">{snap.online ? `${snap.project || "control plane"}${snap.branch ? " · " + snap.branch : ""}` : "offline"}</span>
  </div>

  {#if !snap.online}
    <div class="empty">
      <p>Caldera daemon not reachable on <code>127.0.0.1:4175</code>.</p>
      <p class="sub">Start the Caldera control plane — runs &amp; attention appear here live.</p>
    </div>
  {:else}
    {#if snap.runs.length}
      <div class="sect">Agent Runs <span class="cnt">{snap.runs.length}</span></div>
      {#each snap.runs as r (r.agent + r.step)}
        <div class="run">
          <span class="rdot" style="background:{statusColor(r.status)}"></span>
          <span class="agent">{r.agent}</span>
          <span class="step">{r.step}</span>
          <span class="rstatus" style="color:{statusColor(r.status)}">{r.status}</span>
          {#if r.summary}<span class="summary">{r.summary}</span>{/if}
        </div>
      {/each}
    {/if}
    {#if snap.attention.length}
      <div class="sect">Attention <span class="cnt">{snap.attention.length}</span></div>
      {#each snap.attention as a, i (i)}
        <div class="att">⚠ {a}</div>
      {/each}
    {/if}
    {#if !snap.runs.length && !snap.attention.length}
      <div class="empty"><p>Connected. No active agent runs.</p></div>
    {/if}
  {/if}
</div>

<style>
  .cal { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--bg); overflow-y: auto; }
  .head { height: 30px; flex: 0 0 auto; display: flex; align-items: center; gap: 9px; padding: 0 14px;
    border-bottom: 1px solid var(--border); font-size: 12px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex: 0 0 auto; }
  .ttl { font-weight: 600; color: var(--text); }
  .muted { color: var(--text3); }
  .sect { padding: 11px 14px 5px; font-size: 10px; letter-spacing: 0.08em; text-transform: uppercase;
    font-weight: 600; color: var(--text3); }
  .cnt { margin-left: 4px; }
  .run { display: flex; align-items: center; gap: 9px; padding: 4px 14px; font-size: 12.5px; }
  .rdot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; }
  .agent { color: var(--text); font-family: var(--font-mono); font-weight: 600; }
  .step { color: var(--text2); font-family: var(--font-mono); font-size: 11.5px; }
  .rstatus { font-family: var(--font-mono); font-size: 11px; }
  .summary { color: var(--text3); font-size: 11.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .att { padding: 4px 14px; font-size: 12px; color: var(--yellow); }
  .empty { padding: 24px 16px; color: var(--text3); font-size: 12.5px; }
  .empty .sub { color: var(--text3); opacity: 0.7; margin-top: 6px; }
  code { font-family: var(--font-mono); color: var(--accent); }
</style>
