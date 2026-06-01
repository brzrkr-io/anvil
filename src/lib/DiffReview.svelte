<script lang="ts">
  import { diffLines, applyHunks, type DiffHunk } from "$lib/linediff";

  // Per-hunk accept/reject of an agent-proposed file edit (#54). Diffs the
  // current file against the proposal; the user toggles each hunk, then applies
  // the merged result. Pure diff math lives in linediff.ts.
  let { path, proposed, current, onApply, onCancel }: {
    path: string;
    proposed: string;
    current: string;
    onApply: (path: string, merged: string) => void;
    onCancel: () => void;
  } = $props();

  const hunks: DiffHunk[] = $derived(diffLines(current, proposed));
  let accepted = $state<boolean[]>([]);
  $effect(() => { accepted = hunks.map(() => true); });

  const acceptedCount = $derived(accepted.filter(Boolean).length);
  function toggle(i: number) { accepted = accepted.map((v, k) => (k === i ? !v : v)); }
  function apply() { onApply(path, applyHunks(current, hunks, accepted)); }
</script>

<div class="dr">
  <div class="drhd">
    <span class="t">Review {hunks.length} change{hunks.length === 1 ? "" : "s"}</span>
    <button class="act" disabled={!hunks.length} onclick={apply}>Apply {acceptedCount}/{hunks.length}</button>
    <button class="act ghost" onclick={onCancel}>Cancel</button>
  </div>
  {#if !hunks.length}
    <div class="none">No differences.</div>
  {/if}
  {#each hunks as h, i (i)}
    <div class="hunk" class:rejected={!accepted[i]}>
      <button class="tog" onclick={() => toggle(i)} title={accepted[i] ? "Reject this change" : "Accept this change"}>
        {accepted[i] ? "✓" : "○"}
      </button>
      <pre class="lines">{#each h.oldLines as l}<span class="del">-{l}</span>{/each}{#each h.newLines as l}<span class="add">+{l}</span>{/each}</pre>
    </div>
  {/each}
</div>

<style>
  .dr { border: 1px solid var(--border); border-radius: 8px; margin: 6px 0; overflow: hidden; background: var(--bg); }
  .drhd { display: flex; align-items: center; gap: 8px; padding: 5px 9px; background: var(--panel);
    border-bottom: 1px solid var(--border); }
  .drhd .t { flex: 1; font-size: 11.5px; color: var(--text2); }
  .act { border: 1px solid var(--border); background: var(--accent); color: var(--bg); font-size: 11px;
    padding: 3px 9px; border-radius: 6px; cursor: default; font-family: var(--font-ui); }
  .act.ghost { background: transparent; color: var(--text2); }
  .act:disabled { opacity: 0.4; }
  .none { padding: 8px 10px; color: var(--text3); font-size: 11.5px; }
  .hunk { display: flex; align-items: flex-start; gap: 6px; padding: 4px 6px; border-bottom: 1px solid var(--border); }
  .hunk:last-child { border-bottom: 0; }
  .hunk.rejected { opacity: 0.5; }
  .tog { flex: 0 0 auto; width: 20px; height: 20px; border: 1px solid var(--border); border-radius: 5px;
    background: var(--bg); color: var(--accent); cursor: default; font-size: 12px; }
  .lines { margin: 0; flex: 1; min-width: 0; font-family: var(--font-mono); font-size: 11.5px; line-height: 1.5;
    overflow-x: auto; white-space: pre; }
  .del { display: block; color: var(--red); background: color-mix(in srgb, var(--red) 12%, transparent); }
  .add { display: block; color: var(--green); background: color-mix(in srgb, var(--green) 12%, transparent); }
</style>
