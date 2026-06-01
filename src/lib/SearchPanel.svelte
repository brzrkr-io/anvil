<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { applyLineEdits, groupByFile, type PathLineEdit } from "$lib/search-edit";

  let { root, onOpen }: { root: string; onOpen: (path: string, line: number) => void } = $props();

  interface Hit { path: string; line: number; text: string; }
  let query = $state("");
  let results = $state<Hit[]>([]);
  let busy = $state(false);
  let ran = $state(false);

  // Multi-buffer search-and-edit (#38): in edit mode each match line is editable;
  // Save writes every changed line back into its file. Edits keyed `path:line`.
  let editMode = $state(false);
  let edits = $state<Record<string, string>>({});
  let saveMsg = $state("");
  const editKey = (h: Hit) => `${h.path}:${h.line}`;
  const dirtyCount = $derived(
    results.filter((h) => edits[editKey(h)] !== undefined && edits[editKey(h)] !== h.text).length,
  );
  function setEdit(h: Hit, v: string) { edits = { ...edits, [editKey(h)]: v }; }
  function toggleEdit() { editMode = !editMode; if (!editMode) { edits = {}; saveMsg = ""; } }

  async function saveEdits() {
    const changed: PathLineEdit[] = results
      .filter((h) => edits[editKey(h)] !== undefined && edits[editKey(h)] !== h.text)
      .map((h) => ({ path: h.path, line: h.line, text: edits[editKey(h)] }));
    if (!changed.length) return;
    busy = true;
    let files = 0;
    for (const [path, le] of groupByFile(changed)) {
      try {
        const text = await invoke<string>("read_file", { path });
        await invoke("write_file", { path, contents: applyLineEdits(text, le) });
        files++;
      } catch { /* skip unwritable */ }
    }
    saveMsg = `Saved ${changed.length} edit${changed.length === 1 ? "" : "s"} in ${files} file${files === 1 ? "" : "s"}`;
    edits = {};
    busy = false;
    await run();
  }

  // Virtualized list: only render rows in view + overscan so thousands of
  // matches stay smooth.
  const ROW_H = 40;
  const OVERSCAN = 6;
  let scrollTop = $state(0);
  let viewH = $state(600);
  let viewport: HTMLDivElement | undefined;

  const start = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN));
  const end = $derived(Math.min(results.length, Math.ceil((scrollTop + viewH) / ROW_H) + OVERSCAN));
  const slice = $derived(results.slice(start, end));

  function onScroll() {
    if (viewport) { scrollTop = viewport.scrollTop; viewH = viewport.clientHeight; }
  }

  async function run() {
    ran = true;
    if (!query.trim()) { results = []; return; }
    busy = true;
    try {
      const raw = await invoke<string>("grep", { root, query });
      results = raw
        .split("\n")
        .filter(Boolean)
        .slice(0, 5000)
        .map((l) => {
          const m = l.match(/^(.*?):(\d+):(\d+):(.*)$/);
          return m ? { path: m[1], line: Number(m[2]), text: m[4] } : null;
        })
        .filter((x): x is Hit => x !== null);
    } catch (e) {
      results = [];
    }
    if (viewport) { viewport.scrollTop = 0; scrollTop = 0; }
    busy = false;
  }

  const rel = (p: string) => (p.startsWith(root) ? p.slice(root.length).replace(/^\//, "") : p);

  // Literal replace-all across every matched file (#83). Rewrites the exact
  // query substring → replacement, then re-runs the search.
  let replace = $state("");
  let replaceMsg = $state("");
  async function replaceAll() {
    if (!query || !results.length) return;
    busy = true;
    const paths = [...new Set(results.map((r) => r.path))];
    // Preview (#84): count occurrences + files before touching disk, then confirm.
    let occ = 0, hitFiles = 0;
    const contents = new Map<string, string>();
    for (const p of paths) {
      try {
        const text = await invoke<string>("read_file", { path: p });
        const n = text.split(query).length - 1;
        if (n > 0) { occ += n; hitFiles++; contents.set(p, text); }
      } catch { /* skip unreadable */ }
    }
    busy = false;
    if (!occ) { replaceMsg = "No occurrences"; return; }
    if (!confirm(`Replace ${occ} occurrence${occ === 1 ? "" : "s"} of "${query}" with "${replace}" across ${hitFiles} file${hitFiles === 1 ? "" : "s"}?`)) return;
    busy = true;
    let files = 0;
    for (const [p, text] of contents) {
      try { await invoke("write_file", { path: p, contents: text.split(query).join(replace) }); files++; }
      catch { /* skip unwritable */ }
    }
    replaceMsg = `Replaced ${occ} in ${files} file${files === 1 ? "" : "s"}`;
    busy = false;
    await run();
  }
</script>

<div class="sp">
  <div class="bar">
    <input bind:value={query} onkeydown={(e) => e.key === "Enter" && run()} placeholder="Search workspace (Enter)" spellcheck="false" />
    {#if results.length}<span class="count">{results.length} matches</span>{/if}
  </div>
  <div class="bar">
    <input bind:value={replace} onkeydown={(e) => e.key === "Enter" && replaceAll()} placeholder="Replace with… (literal)" spellcheck="false" />
    <button class="rep" disabled={busy || !results.length} onclick={replaceAll}>Replace All</button>
    {#if replaceMsg}<span class="count">{replaceMsg}</span>{/if}
  </div>
  <div class="bar">
    <button class="rep {editMode ? 'on' : ''}" disabled={!results.length} onclick={toggleEdit}>{editMode ? "Editing rows" : "Edit results"}</button>
    {#if editMode}
      <button class="rep" disabled={busy || dirtyCount === 0} onclick={saveEdits}>Save {dirtyCount || ""} edit{dirtyCount === 1 ? "" : "s"}</button>
    {/if}
    {#if saveMsg}<span class="count">{saveMsg}</span>{/if}
  </div>
  <div class="res" bind:this={viewport} onscroll={onScroll}>
    {#if busy}<div class="msg">Searching…</div>{/if}
    {#if !busy && ran && results.length === 0}<div class="msg">No matches</div>{/if}
    {#if results.length}
      <div class="spacer" style="height:{results.length * ROW_H}px">
        {#each slice as r, i (r.path + r.line + r.text + (start + i))}
          <div class="hit" style="top:{(start + i) * ROW_H}px;height:{ROW_H}px"
               onclick={() => !editMode && onOpen(r.path, r.line)} role="button" tabindex="0">
            <span class="loc mono">{rel(r.path)}:{r.line}{#if editMode && edits[editKey(r)] !== undefined && edits[editKey(r)] !== r.text}<span class="dot">●</span>{/if}</span>
            {#if editMode}
              <input class="edit mono" value={edits[editKey(r)] ?? r.text} spellcheck="false"
                     oninput={(e) => setEdit(r, (e.currentTarget as HTMLInputElement).value)} />
            {:else}
              <span class="txt mono">{r.text.trim().slice(0, 200)}</span>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .sp { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .bar { display: flex; align-items: center; gap: 8px; padding: 10px; border-bottom: 1px solid var(--border); }
  .bar input {
    flex: 1; padding: 7px 10px; border: 1px solid var(--border); border-radius: 8px;
    background: var(--bg); color: var(--text); font-size: 13px; outline: 0;
  }
  .count { flex: 0 0 auto; color: var(--text3); font-size: 11px; }
  .rep { flex: 0 0 auto; border: 1px solid var(--border); background: var(--bg); color: var(--text2);
    font-family: var(--font-ui); font-size: 11.5px; padding: 4px 10px; border-radius: 6px; cursor: default; }
  .rep:hover:not(:disabled) { border-color: var(--accent); color: var(--text); }
  .rep:disabled { opacity: 0.4; }
  .rep.on { border-color: var(--accent); color: var(--accent); }
  .edit { display: block; width: 100%; margin-top: 1px; padding: 1px 4px; border: 1px solid var(--border);
    border-radius: 4px; background: var(--bg); color: var(--text); font-size: 12px; outline: 0; box-sizing: border-box; }
  .edit:focus { border-color: var(--accent); }
  .dot { color: var(--accent); margin-left: 5px; font-size: 9px; vertical-align: middle; }
  .res { flex: 1; overflow-y: auto; padding: 4px 0; position: relative; }
  .spacer { position: relative; width: 100%; }
  .hit { position: absolute; left: 0; right: 0; padding: 5px 12px; cursor: default; box-sizing: border-box; }
  .hit:hover { background: var(--panel); }
  .loc { display: block; color: var(--accent); font-size: 11px; }
  .txt { display: block; color: var(--text2); font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .msg { padding: 16px 12px; color: var(--text3); font-size: 12.5px; }
</style>
