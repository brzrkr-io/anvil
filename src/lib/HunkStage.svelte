<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { parseHunks, buildHunkPatch, buildLinePatch, type FileDiff } from "$lib/git";
  import Icon from "$lib/Icon.svelte";

  // Inline per-hunk stage/discard (#62). For an unstaged file we show its
  // working-tree diff split into hunks; staged files show the cached diff with
  // an Unstage action per hunk.
  let { cwd, path, staged = false, onChanged }: {
    cwd: string;
    path: string;
    staged?: boolean;
    onChanged?: () => void;
  } = $props();

  let file = $state<FileDiff>({ preamble: "", hunks: [] });
  let busy = $state(false);
  let err = $state("");

  async function load() {
    try {
      const diff = await invoke<string>("git_diff", { cwd, path, staged });
      file = parseHunks(diff);
    } catch (e) {
      err = String(e);
    }
  }
  $effect(() => {
    void cwd; void path; void staged;
    load();
  });

  // Stage a hunk: apply --cached. Unstage: apply --cached --reverse of the
  // cached diff. Discard: apply --reverse to the working tree.
  async function apply(i: number, mode: "stage" | "unstage" | "discard") {
    busy = true;
    err = "";
    const patch = buildHunkPatch(file, i);
    try {
      await invoke("git_apply_hunk", {
        cwd,
        patch,
        cached: mode !== "discard",
        reverse: mode !== "stage",
      });
      onChanged?.();
      await load();
    } catch (e) {
      err = String(e);
    }
    busy = false;
  }

  function lineClass(l: string): string {
    if (l.startsWith("+")) return "add";
    if (l.startsWith("-")) return "del";
    if (l.startsWith("@@")) return "hdr";
    return "ctx";
  }

  // #22 Stage-by-line: per-hunk set of selected body-line indices.
  let sel = $state<Record<number, Set<number>>>({});
  function toggleLine(hi: number, li: number, l: string) {
    if (l[0] !== "+" && l[0] !== "-") return; // only changed lines selectable
    const s = new Set(sel[hi] ?? []);
    if (s.has(li)) s.delete(li); else s.add(li);
    sel = { ...sel, [hi]: s };
  }
  async function stageSelected(hi: number) {
    const s = sel[hi];
    if (!s?.size) return;
    const patch = buildLinePatch(file, hi, s);
    if (!patch) return;
    busy = true; err = "";
    try {
      await invoke("git_apply_hunk", { cwd, patch, cached: true, reverse: false });
      sel = { ...sel, [hi]: new Set() };
      onChanged?.();
      await load();
    } catch (e) { err = String(e); }
    busy = false;
  }
</script>

<div class="hunks">
  {#if err}<div class="err">{err}</div>{/if}
  {#if !file.hunks.length}
    <div class="empty">No hunks</div>
  {/if}
  {#each file.hunks as h, i (i)}
    <div class="hunk">
      <div class="bar">
        <span class="loc">{h.header.replace(/^@@ | @@.*$/g, "")}</span>
        {#if staged}
          <button class="op" disabled={busy} title="Unstage hunk" onclick={() => apply(i, "unstage")}><Icon name="minus" size={12} /></button>
        {:else}
          {#if sel[i]?.size}<button class="op stage" disabled={busy} title="Stage {sel[i].size} selected line(s)" onclick={() => stageSelected(i)}>+{sel[i].size}</button>{/if}
          <button class="op danger" disabled={busy} title="Discard hunk" onclick={() => apply(i, "discard")}><Icon name="close" size={12} /></button>
          <button class="op" disabled={busy} title="Stage hunk" onclick={() => apply(i, "stage")}><Icon name="plus" size={12} /></button>
        {/if}
      </div>
      <pre class="code">{#each h.body.split("\n").slice(1) as l, li}<span class="ln {lineClass(l)} {!staged && (l[0] === '+' || l[0] === '-') ? 'sel-able' : ''} {sel[i]?.has(li + 1) ? 'picked' : ''}" onclick={() => !staged && toggleLine(i, li + 1, l)}>{l || " "}</span>{/each}</pre>
    </div>
  {/each}
</div>

<style>
  .hunks { padding: 2px 0 6px; }
  .hunk { border: 1px solid var(--border); border-radius: 7px; margin: 4px 14px; overflow: hidden; }
  .bar { display: flex; align-items: center; gap: 4px; height: 24px; padding: 0 8px;
    background: var(--panel); border-bottom: 1px solid var(--border); }
  .loc { flex: 1; min-width: 0; font-family: var(--font-mono); font-size: 10.5px; color: var(--text3);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .op { width: 20px; height: 18px; border: 0; border-radius: 5px; background: transparent;
    color: var(--text3); cursor: default; display: inline-flex; align-items: center; justify-content: center; }
  .op:hover { background: var(--sel); color: var(--text); }
  .op.danger:hover { color: var(--red); }
  .op:disabled { opacity: 0.4; }
  .code { margin: 0; padding: 4px 0; font-family: var(--font-mono); font-size: 11.5px; line-height: 1.45;
    overflow-x: auto; white-space: pre; }
  .ln { display: block; padding: 0 8px; }
  .ln.add { background: color-mix(in srgb, var(--green) 16%, transparent); color: var(--text); }
  .ln.del { background: color-mix(in srgb, var(--red) 16%, transparent); color: var(--text); }
  .ln.ctx { color: var(--text2); }
  .ln.sel-able { cursor: default; }
  .ln.sel-able:hover { outline: 1px solid var(--border); outline-offset: -1px; }
  .ln.picked { box-shadow: inset 2px 0 0 var(--accent); }
  .op.stage { width: auto; padding: 0 6px; font-family: var(--font-mono); font-size: 10px; color: var(--accent); }
  .err { padding: 6px 14px; color: var(--red); font-size: 11px; font-family: var(--font-mono); }
  .empty { padding: 8px 14px; color: var(--text3); font-size: 11.5px; }
</style>
