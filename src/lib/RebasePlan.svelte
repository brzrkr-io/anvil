<script lang="ts">
  // #21 Interactive rebase editor — pick / fixup / drop + reorder the commits in
  // `target..HEAD`, then run a non-interactive rebase from the built todo.
  import { invoke } from "@tauri-apps/api/core";
  let { cwd, target, onClose, onDone }: { cwd: string; target: string; onClose: () => void; onDone: () => void } = $props();

  type Row = { sha: string; subject: string; action: "pick" | "fixup" | "drop" };
  let rows = $state<Row[]>([]);
  let err = $state("");
  let busy = $state(false);

  $effect(() => {
    (async () => {
      try {
        const log = await invoke<string>("git_log_range", { cwd, range: `${target}..HEAD` });
        // git log is newest-first; rebase todo is oldest-first.
        rows = log.split("\n").filter(Boolean).reverse().map((l) => {
          const sp = l.indexOf(" ");
          return { sha: l.slice(0, sp), subject: l.slice(sp + 1), action: "pick" as const };
        });
        if (!rows.length) err = `Nothing to rebase — HEAD is at or behind ${target}.`;
      } catch (e) { err = String(e); }
    })();
  });

  function move(i: number, d: -1 | 1) {
    const j = i + d;
    if (j < 0 || j >= rows.length) return;
    const next = [...rows];
    [next[i], next[j]] = [next[j], next[i]];
    rows = next;
  }
  const ACTIONS: Row["action"][] = ["pick", "fixup", "drop"];

  async function run() {
    const todo = rows.map((r) => (r.action === "drop" ? `drop ${r.sha} ${r.subject}` : `${r.action} ${r.sha} ${r.subject}`)).join("\n") + "\n";
    if (!rows.some((r) => r.action === "pick")) { err = "At least one commit must be 'pick'."; return; }
    busy = true; err = "";
    try { await invoke("git_rebase_run", { cwd, target, todo }); onDone(); onClose(); }
    catch (e) { err = String(e).slice(0, 400); }
    busy = false;
  }
</script>

<div class="rb-scrim" role="presentation" onclick={onClose}>
  <div class="rb" role="dialog" aria-modal="true" aria-label="Interactive rebase" onclick={(e) => e.stopPropagation()}>
    <header class="rb-head"><h2>Interactive rebase onto <code>{target}</code></h2><button class="rb-x" onclick={onClose}>✕</button></header>
    {#if err}<div class="rb-err">{err}</div>{/if}
    <div class="rb-rows">
      {#each rows as r, i (r.sha)}
        <div class="rb-row" class:drop={r.action === "drop"}>
          <div class="rb-ord"><button onclick={() => move(i, -1)} disabled={i === 0} title="Up">↑</button><button onclick={() => move(i, 1)} disabled={i === rows.length - 1} title="Down">↓</button></div>
          <select bind:value={r.action}>{#each ACTIONS as a}<option value={a}>{a}</option>{/each}</select>
          <code class="rb-sha">{r.sha}</code>
          <span class="rb-sub">{r.subject}</span>
        </div>
      {/each}
    </div>
    <footer class="rb-foot">
      <span class="rb-hint">pick keeps · fixup melts into the previous · drop removes · reorder with ↑↓</span>
      <span style="flex:1"></span>
      <button class="rb-cancel" onclick={onClose}>Cancel</button>
      <button class="rb-go" disabled={busy || !rows.length} onclick={run}>{busy ? "Rebasing…" : "Start rebase"}</button>
    </footer>
  </div>
</div>

<style>
  .rb-scrim { position: fixed; inset: 0; background: var(--glass-scrim); backdrop-filter: blur(6px); -webkit-backdrop-filter: blur(6px); z-index: 200; display: flex; align-items: center; justify-content: center; }
  .rb { width: 720px; max-width: 94vw; max-height: 82vh; display: flex; flex-direction: column; background: var(--glass); backdrop-filter: blur(var(--blur)) saturate(1.3); -webkit-backdrop-filter: blur(var(--blur)) saturate(1.3); border: 1px solid var(--border); border-radius: 8px; overflow: hidden; box-shadow: var(--elev-3), inset 0 1px 0 var(--hairline); }
  .rb-head { display: flex; align-items: center; padding: 12px 16px; border-bottom: 1px solid var(--border); }
  .rb-head h2 { margin: 0; flex: 1; font-size: 14px; font-weight: 600; color: var(--text); }
  .rb-head code, .rb-sha { font-family: var(--font-mono); color: var(--accent); }
  .rb-x { border: 0; background: transparent; color: var(--text3); font-size: 14px; cursor: default; }
  .rb-err { padding: 8px 16px; color: var(--red); font-size: 12px; font-family: var(--font-mono); white-space: pre-wrap; }
  .rb-rows { overflow-y: auto; padding: 8px; }
  .rb-row { display: flex; align-items: center; gap: 8px; padding: 4px 8px; border-radius: 6px; }
  .rb-row:hover { background: var(--sel); }
  .rb-row.drop { opacity: 0.5; }
  .rb-ord { display: flex; flex-direction: column; }
  .rb-ord button { border: 0; background: transparent; color: var(--text3); cursor: default; font-size: 9px; line-height: 1; padding: 1px; }
  .rb-ord button:disabled { opacity: 0.3; }
  .rb-row select { background: var(--panel2); color: var(--text); border: 1px solid var(--border); border-radius: 5px; padding: 2px 4px; font-size: 11px; }
  .rb-sha { font-size: 11px; }
  .rb-sub { flex: 1; min-width: 0; font-size: 12.5px; color: var(--text2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rb-foot { display: flex; align-items: center; gap: 10px; padding: 10px 16px; border-top: 1px solid var(--border); }
  .rb-hint { font-size: 11px; color: var(--text3); }
  .rb-cancel { border: 0; background: transparent; color: var(--text2); font-size: 13px; padding: 7px 12px; cursor: default; }
  .rb-go { border: 0; background: var(--accent); color: var(--bg); font-size: 13px; font-weight: 500; padding: 7px 16px; border-radius: 8px; cursor: default; }
  .rb-go:disabled { opacity: 0.5; }
</style>
