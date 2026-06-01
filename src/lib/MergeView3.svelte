<script lang="ts">
  // #25 3-pane merge view — base (:1) / ours (:2) / theirs (:3) for a conflicted
  // file, read-only side-by-side so you can compare while resolving in the main
  // editor. (Interactive merge-into-result is a future extension.)
  import { invoke } from "@tauri-apps/api/core";
  let { cwd, path, onClose, onResolved }: { cwd: string; path: string; onClose: () => void; onResolved?: () => void } = $props();
  let resolveErr = $state("");
  async function takeSide(side: "ours" | "theirs") {
    resolveErr = "";
    try { await invoke("git_checkout_side", { cwd, path, side }); onResolved?.(); onClose(); }
    catch (e) { resolveErr = String(e).slice(0, 200); }
  }
  async function markResolved() {
    resolveErr = "";
    try { await invoke("git_stage", { path }); onResolved?.(); onClose(); }
    catch (e) { resolveErr = String(e).slice(0, 200); }
  }

  let base = $state("");
  let ours = $state("");
  let theirs = $state("");
  let err = $state("");

  async function stage(rev: string): Promise<string> {
    try { return await invoke<string>("git_show_file", { cwd, rev, path }); } catch { return ""; }
  }
  $effect(() => {
    (async () => {
      const [b, o, t] = await Promise.all([stage(":1"), stage(":2"), stage(":3")]);
      base = b; ours = o; theirs = t;
      if (!o && !t) err = "No merge conflict stages for this file (nothing to compare).";
    })();
  });

  const cols = $derived([
    { title: "Base", sub: "common ancestor", text: base },
    { title: "Ours", sub: "HEAD", text: ours },
    { title: "Theirs", sub: "incoming", text: theirs },
  ]);
</script>

<div class="mv-scrim" role="presentation" onclick={onClose}>
  <div class="mv" role="dialog" aria-modal="true" aria-label="3-pane merge" onclick={(e) => e.stopPropagation()}>
    <header class="mv-head">
      <h2>3-Pane Merge · {path.split("/").pop()}</h2>
      <button class="mv-x" onclick={onClose}>✕</button>
    </header>
    {#if err}
      <div class="mv-empty">{err}</div>
    {:else}
      {#if resolveErr}<div class="mv-rerr">{resolveErr}</div>{/if}
      <div class="mv-actions">
        <span class="mv-hint">Resolve the conflict, or take a side wholesale:</span>
        <span style="flex:1"></span>
        <button onclick={() => takeSide("ours")}>Take Ours</button>
        <button onclick={() => takeSide("theirs")}>Take Theirs</button>
        <button class="mv-done" onclick={markResolved}>Mark Resolved (stage)</button>
      </div>
      <div class="mv-cols">
        {#each cols as c}
          <section class="mv-col">
            <div class="mv-ct"><b>{c.title}</b> <span>{c.sub}</span></div>
            <pre class="mv-body">{c.text || "(empty)"}</pre>
          </section>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .mv-scrim { position: fixed; inset: 0; background: var(--glass-scrim); backdrop-filter: blur(6px); -webkit-backdrop-filter: blur(6px); z-index: 200; display: flex; align-items: center; justify-content: center; }
  .mv { width: 94vw; height: 86vh; display: flex; flex-direction: column; background: var(--glass); backdrop-filter: blur(var(--blur)) saturate(1.3); -webkit-backdrop-filter: blur(var(--blur)) saturate(1.3); border: 1px solid var(--border); border-radius: 8px; overflow: hidden; box-shadow: var(--elev-3), inset 0 1px 0 var(--hairline); }
  .mv-head { display: flex; align-items: center; padding: 10px 14px; border-bottom: 1px solid var(--border); }
  .mv-head h2 { margin: 0; flex: 1; font-size: 14px; font-weight: 600; color: var(--text); }
  .mv-x { border: 0; background: transparent; color: var(--text3); font-size: 14px; cursor: default; }
  .mv-x:hover { color: var(--text); }
  .mv-cols { flex: 1; min-height: 0; display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 1px; background: var(--border); }
  .mv-col { display: flex; flex-direction: column; min-width: 0; background: var(--bg); }
  .mv-ct { padding: 6px 10px; font-size: 11px; color: var(--text3); border-bottom: 1px solid var(--border); }
  .mv-ct b { color: var(--text2); }
  .mv-body { flex: 1; min-height: 0; overflow: auto; margin: 0; padding: 8px 10px; font-family: var(--font-mono); font-size: 11.5px; line-height: 1.5; color: var(--text2); white-space: pre; }
  .mv-empty { padding: 40px; text-align: center; color: var(--text3); }
  .mv-actions { display: flex; align-items: center; gap: 8px; padding: 8px 14px; border-bottom: 1px solid var(--border); }
  .mv-hint { font-size: 11px; color: var(--text3); }
  .mv-actions button { border: 1px solid var(--border); background: var(--panel2); color: var(--text); border-radius: 6px; padding: 4px 10px; font-size: 12px; cursor: default; }
  .mv-actions button:hover { border-color: var(--accent); }
  .mv-actions .mv-done { background: var(--accent); color: var(--bg); border-color: transparent; }
  .mv-rerr { padding: 6px 14px; color: var(--red); font-size: 11px; font-family: var(--font-mono); }
</style>
