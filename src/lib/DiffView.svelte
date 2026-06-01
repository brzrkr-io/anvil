<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { EditorState, Compartment } from "@codemirror/state";
  import { EditorView, lineNumbers } from "@codemirror/view";
  import { unifiedMergeView } from "@codemirror/merge";
  import { cmTheme } from "$lib/cm-theme";
  import { cmLang } from "$lib/cm-lang";
  import { activeTheme, themes } from "$lib/themes";

  let {
    cwd,
    path,
    staged,
    rev,
  }: { cwd: string; path?: string; staged?: boolean; rev?: string } = $props();

  let host: HTMLDivElement;
  let view: EditorView | undefined;
  let unsub: () => void;
  let empty = $state(false);
  const themeComp = new Compartment();

  // Diff-chunk colors layered over the palette (subtle line tint + stronger
  // intra-line "changed text" highlight — the Terax word-diff look).
  function diffTheme(name: string) {
    const ui = themes[name]?.ui ?? themes["solarized-dark"].ui;
    return EditorView.theme({
      ".cm-changedLine": { backgroundColor: ui.green + "1f" },
      ".cm-changedText": { backgroundColor: ui.green + "59", borderRadius: "2px" },
      ".cm-deletedChunk": { backgroundColor: ui.red + "1f" },
      ".cm-deletedChunk .cm-deletedText, .cm-deletedText": { backgroundColor: ui.red + "59", borderRadius: "2px" },
      ".cm-insertedLine": { backgroundColor: ui.green + "1f" },
      ".cm-collapsedLines": { color: ui.text3, backgroundColor: ui.panel + "88", padding: "2px 0" },
    });
  }

  async function refresh() {
    if (!host) return;
    view?.destroy();
    view = undefined;

    const rel = path && path.startsWith(cwd) ? path.slice(cwd.length).replace(/^\//, "") : path;

    let original = "";
    let modified = "";
    if (rev && rel) {
      try { original = await invoke<string>("git_show_file", { cwd, rev: `${rev}~1`, path: rel }); } catch { original = ""; }
      try { modified = await invoke<string>("git_show_file", { cwd, rev, path: rel }); } catch { modified = ""; }
    } else if (rev) {
      // Whole-commit fallback: show the patch as text (no per-file original).
      try { modified = await invoke<string>("git_show", { cwd, rev }); } catch { modified = ""; }
      original = modified;
    } else if (rel) {
      try { original = await invoke<string>("git_show_file", { cwd, rev: "HEAD", path: rel }); } catch { original = ""; }
      try { modified = await invoke<string>("read_file", { path: path! }); } catch { modified = ""; }
    }

    empty = original === modified;
    if (empty) return;

    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: modified,
        extensions: [
          lineNumbers(),
          EditorState.readOnly.of(true),
          EditorView.editable.of(false),
          ...cmLang(path ?? ""),
          themeComp.of([cmTheme($activeTheme), diffTheme($activeTheme)]),
          unifiedMergeView({
            original,
            mergeControls: false,
            gutter: true,
            highlightChanges: true,
            syntaxHighlightDeletions: true,
            collapseUnchanged: { margin: 3, minSize: 4 },
          }),
        ],
      }),
    });
  }

  onMount(() => {
    refresh();
    unsub = activeTheme.subscribe((n) => view?.dispatch({ effects: themeComp.reconfigure([cmTheme(n), diffTheme(n)]) }));
  });

  $effect(() => {
    void [cwd, path, staged, rev];
    refresh();
  });

  onDestroy(() => {
    unsub?.();
    view?.destroy();
  });
</script>

<div class="root">
  <div class="ed" class:hidden={empty} bind:this={host}></div>
  {#if empty}
    <div class="empty">No changes</div>
  {/if}
</div>

<style>
  .root { width: 100%; height: 100%; min-height: 0; position: relative; display: flex; }
  .ed { width: 100%; height: 100%; min-height: 0; overflow: hidden; }
  .ed :global(.cm-editor) { height: 100%; }
  .ed :global(.cm-editor.cm-focused) { outline: none; }
  .ed.hidden { display: none; }
  .empty {
    width: 100%; height: 100%; display: flex; align-items: center; justify-content: center;
    color: var(--text3); font-size: 13px;
  }
</style>
