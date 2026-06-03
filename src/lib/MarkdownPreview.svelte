<script lang="ts">
  import { marked } from "marked";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import { editorLive } from "$lib/editor-live";

  // Rendered Markdown preview (#6). Reflects the live editor buffer for the open
  // file (unsaved edits included) and falls back to disk + a light poll for files
  // that aren't the active editor. Local-file trust model.
  let { path }: { path: string } = $props();
  let html = $state("");
  let timer: ReturnType<typeof setInterval> | undefined;

  async function render(text: string) {
    const out = await marked.parse(text, { gfm: true, breaks: false });
    html = out.replace(/<script[\s\S]*?<\/script>/gi, "");
  }
  async function refresh() {
    const live = $editorLive;
    if (live && live.path === path) { await render(live.text); return; }
    try { await render(await invoke<string>("read_file", { path })); } catch { html = ""; }
  }
  // Live-reload poll for external edits — skip while the window is backgrounded
  // so a hidden preview does no file reads. The live editor buffer (editorLive)
  // updates instantly regardless.
  onMount(() => { refresh(); timer = setInterval(() => { if (typeof document === "undefined" || !document.hidden) refresh(); }, 1500); });
  onDestroy(() => { if (timer) clearInterval(timer); });
  // Re-render on path change AND whenever the live buffer for this path updates.
  $effect(() => { void path; const live = $editorLive; if (live && live.path === path) render(live.text); else refresh(); });
</script>

<div class="mdp">
  <!-- eslint-disable-next-line svelte/no-at-html-tags -->
  <article class="md">{@html html}</article>
</div>

<style>
  .mdp { width: 100%; height: 100%; min-height: 0; overflow-y: auto; background: var(--bg); }
  .md { max-width: 820px; margin: 0 auto; padding: 28px 32px; color: var(--text); font-family: var(--font-ui);
    font-size: 14px; line-height: 1.65; }
  .md :global(h1), .md :global(h2), .md :global(h3) { color: var(--text); font-weight: 600; line-height: 1.3;
    margin: 1.4em 0 0.5em; }
  .md :global(h1) { font-size: 1.7em; border-bottom: 1px solid var(--border); padding-bottom: 0.25em; }
  .md :global(h2) { font-size: 1.4em; border-bottom: 1px solid var(--border); padding-bottom: 0.2em; }
  .md :global(h3) { font-size: 1.18em; }
  .md :global(a) { color: var(--accent); text-decoration: none; }
  .md :global(a:hover) { text-decoration: underline; }
  .md :global(p), .md :global(ul), .md :global(ol), .md :global(blockquote), .md :global(table) { margin: 0.7em 0; }
  .md :global(code) { font-family: var(--font-mono); font-size: 0.88em; background: var(--panel2);
    padding: 1px 5px; border-radius: 4px; }
  .md :global(pre) { background: var(--panel); border: 1px solid var(--border); border-radius: 8px;
    padding: 12px 14px; overflow-x: auto; }
  .md :global(pre code) { background: none; padding: 0; }
  .md :global(blockquote) { border-left: 3px solid var(--border); padding-left: 14px; color: var(--text2); }
  .md :global(table) { border-collapse: collapse; width: 100%; }
  .md :global(th), .md :global(td) { border: 1px solid var(--border); padding: 6px 10px; text-align: left; }
  .md :global(th) { background: var(--panel); }
  .md :global(img) { max-width: 100%; border-radius: 6px; }
  .md :global(hr) { border: 0; border-top: 1px solid var(--border); margin: 1.5em 0; }
  .md :global(li) { margin: 0.25em 0; }
</style>
