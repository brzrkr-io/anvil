<script lang="ts">
  // Connections doctor: shows what's installed + authenticated for each
  // integration, with a one-click terminal fix. Read-only; auth itself stays with
  // each tool's native login flow.
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";

  let { onClose, onRunCommand }: { onClose: () => void; onRunCommand?: (cmd: string) => void } = $props();

  interface Probe {
    id: string; label: string; installed: boolean; version: string;
    detail: string; ok: boolean; note: string; fix_cmd: string; fix_label: string;
  }
  let probes = $state<Probe[]>([]);
  let loading = $state(true);
  let err = $state("");

  async function check() {
    loading = true; err = "";
    try { probes = await invoke<Probe[]>("doctor_check"); }
    catch (e) { err = String(e); }
    finally { loading = false; }
  }
  check();

  function fix(p: Probe) {
    if (!p.fix_cmd) return;
    onRunCommand?.(p.fix_cmd);
    onClose();
  }

  const ICONS: Record<string, string> = {
    kubectl: "kube", aws: "devops", gh: "pr", glab: "ci",
    flux: "flux", helm: "helm", terraform: "terraform", docker: "docker",
  };
  const dotColor = (p: Probe) =>
    p.ok ? "var(--status-verified, var(--green, #3fb950))"
      : !p.installed ? "var(--text3)"
      : "var(--status-attention, var(--yellow, #d8a657))";

  const okCount = $derived(probes.filter((p) => p.ok).length);
  // Missing language servers → offer a single "install everything" action that
  // chains each install command in one terminal pane (`;` so one failure doesn't
  // abort the rest).
  const missingServers = $derived(probes.filter((p) => p.id.startsWith("lsp-") && !p.ok && p.fix_cmd));
  function installAllServers() {
    if (!missingServers.length) return;
    onRunCommand?.(missingServers.map((p) => p.fix_cmd).join(" ; "));
    onClose();
  }
</script>

<div class="dr-scrim" onclick={onClose} role="presentation"></div>
<div class="dr" role="dialog" aria-label="Connections">
  <div class="dr-head">
    <div class="dr-titles">
      <span class="dr-title">Connections</span>
      <span class="dr-sub">{loading ? "checking…" : `${okCount}/${probes.length} ready`} · tools &amp; auth for your environment</span>
    </div>
    <span class="spacer"></span>
    {#if missingServers.length}
      <button class="dr-btn primary" onclick={installAllServers} title={missingServers.map((p) => p.fix_cmd).join(" ; ")}>Install {missingServers.length} server{missingServers.length > 1 ? "s" : ""}</button>
    {/if}
    <button class="dr-btn" onclick={check} disabled={loading}>{loading ? "…" : "Re-check"}</button>
    <button class="dr-x" onclick={onClose} title="Close"><Icon name="close" size={13} /></button>
  </div>

  {#if err}<div class="dr-err">{err}</div>{/if}

  <div class="dr-list">
    {#each probes as p, i (p.id + '#' + i)}
      <div class="dr-row" class:bad={!p.ok}>
        <span class="dr-dot" style="background:{dotColor(p)}"></span>
        <span class="dr-ic"><Icon name={ICONS[p.id] ?? "info"} size={15} /></span>
        <div class="dr-text">
          <span class="dr-name">{p.label}</span>
          <span class="dr-meta">
            {#if !p.installed}not installed{:else}{p.detail || p.version || "ready"}{/if}
            {#if p.note}<span class="dr-note"> — {p.note}</span>{/if}
          </span>
        </div>
        <span class="spacer"></span>
        {#if !p.ok && p.fix_cmd}
          <button class="dr-fix" onclick={() => fix(p)} title={`Run: ${p.fix_cmd}`}>{p.fix_label}</button>
        {/if}
      </div>
    {/each}
    {#if loading && !probes.length}<div class="dr-empty">Checking your tools…</div>{/if}
  </div>

  <div class="dr-foot">Fixes run in your terminal. Anvil never stores cloud credentials — each tool keeps its own login.</div>
</div>

<style>
  .dr-scrim { position: fixed; inset: 0; z-index: 90; background: var(--glass-scrim, rgba(0,0,0,0.4));
    backdrop-filter: blur(4px); -webkit-backdrop-filter: blur(4px); }
  .dr { position: fixed; z-index: 91; top: 50%; left: 50%; transform: translate(-50%, -50%);
    width: 560px; max-width: 92vw; max-height: 80vh; display: flex; flex-direction: column;
    background: var(--panel); border: 1px solid var(--border); border-radius: 10px;
    box-shadow: var(--elev-3, 0 12px 40px rgba(0,0,0,0.45)); font-family: var(--font-ui); overflow: hidden; }
  .dr-head { display: flex; align-items: center; gap: 10px; padding: 14px 16px 12px; border-bottom: 1px solid var(--hairline); }
  .dr-titles { display: flex; flex-direction: column; gap: 2px; }
  .dr-title { font-size: 15px; font-weight: 600; color: var(--text); }
  .dr-sub { font-size: 11px; color: var(--text3); }
  .spacer { flex: 1; }
  .dr-btn { font-family: var(--font-ui); font-size: 11.5px; color: var(--text2); background: var(--panel2);
    border: 1px solid var(--border); border-radius: 6px; padding: 4px 10px; cursor: default; }
  .dr-btn:hover:not(:disabled) { color: var(--text); border-color: var(--accent); }
  .dr-btn:disabled { opacity: 0.5; }
  .dr-btn.primary { color: var(--bg); background: var(--accent); border-color: var(--accent); font-weight: 600; }
  .dr-btn.primary:hover { filter: brightness(1.08); }
  .dr-x { background: none; border: 0; color: var(--text3); cursor: default; padding: 2px; display: inline-flex; }
  .dr-x:hover { color: var(--text); }
  .dr-err { margin: 10px 16px 0; padding: 8px 10px; font-size: 12px; color: var(--status-failure, #e5484d);
    background: color-mix(in srgb, var(--status-failure, #e5484d) 10%, transparent); border-radius: 6px; }
  .dr-list { flex: 1; min-height: 0; overflow-y: auto; padding: 6px 8px; }
  .dr-row { display: flex; align-items: center; gap: 10px; padding: 9px 8px; border-radius: 7px; }
  .dr-row:hover { background: color-mix(in srgb, var(--text) 4%, transparent); }
  .dr-dot { width: 8px; height: 8px; border-radius: 50%; flex: 0 0 auto; }
  .dr-ic { color: var(--text2); flex: 0 0 auto; display: inline-flex; }
  .dr-text { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .dr-name { font-size: 13px; color: var(--text); }
  .dr-meta { font-size: 11px; color: var(--text3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 360px; }
  .dr-note { color: var(--status-attention, #d8a657); }
  .dr-fix { font-family: var(--font-ui); font-size: 11.5px; color: var(--accent); background: transparent;
    border: 1px solid var(--accent); border-radius: 6px; padding: 4px 11px; cursor: default; flex: 0 0 auto; white-space: nowrap; }
  .dr-fix:hover { background: color-mix(in srgb, var(--accent) 14%, transparent); }
  .dr-empty { padding: 24px; text-align: center; color: var(--text3); font-size: 12px; }
  .dr-foot { padding: 10px 16px; border-top: 1px solid var(--hairline); font-size: 10.5px; color: var(--text3); }
</style>
