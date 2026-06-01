<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { marked } from "marked";

  // Runbook runner (#62): a Markdown doc whose fenced shell blocks become
  // one-click "Run" steps sent to the terminal. Prose renders as Markdown.
  let { path, onRun }: { path: string; onRun: (cmd: string) => void } = $props();

  type Seg = { code: boolean; lang: string; text: string };
  let segs = $state<Seg[]>([]);
  let ran = $state<Set<number>>(new Set());
  const RUNNABLE = new Set(["bash", "sh", "shell", "zsh", "console", ""]);

  function parse(md: string): Seg[] {
    const out: Seg[] = [];
    const re = /```(\w*)\n?([\s\S]*?)```/g;
    let last = 0;
    let m: RegExpExecArray | null;
    while ((m = re.exec(md))) {
      if (m.index > last) out.push({ code: false, lang: "", text: md.slice(last, m.index) });
      out.push({ code: true, lang: m[1] || "", text: m[2].replace(/\n$/, "") });
      last = re.lastIndex;
    }
    if (last < md.length) out.push({ code: false, lang: "", text: md.slice(last) });
    return out;
  }

  async function load() {
    try { segs = parse(await invoke<string>("read_file", { path })); } catch { segs = []; }
  }
  onMount(load);
  $effect(() => { void path; load(); });

  function runStep(i: number, cmd: string) { onRun(cmd.trim()); ran = new Set(ran).add(i); }
  function runAll() {
    segs.forEach((s, i) => { if (s.code && RUNNABLE.has(s.lang.toLowerCase())) runStep(i, s.text); });
  }
  const proseHtml = (t: string) => marked.parse(t, { gfm: true }) as string;
</script>

<div class="rb">
  <div class="rbbar">
    <span class="t">Runbook · {path.split("/").pop()}</span>
    <button class="runall" onclick={runAll}>Run all steps</button>
  </div>
  <div class="rbody">
    {#each segs as s, i (i)}
      {#if s.code && RUNNABLE.has(s.lang.toLowerCase())}
        <div class="step {ran.has(i) ? 'done' : ''}">
          <button class="run" onclick={() => runStep(i, s.text)} title="Run in terminal">{ran.has(i) ? "✓ run" : "▶ run"}</button>
          <pre class="cmd">{s.text}</pre>
        </div>
      {:else if s.code}
        <pre class="cmd plain">{s.text}</pre>
      {:else if s.text.trim()}
        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
        <div class="prose">{@html proseHtml(s.text)}</div>
      {/if}
    {/each}
    {#if !segs.length}<div class="empty">Empty / not a Markdown runbook.</div>{/if}
  </div>
</div>

<style>
  .rb { width: 100%; height: 100%; min-height: 0; display: flex; flex-direction: column; background: var(--bg); }
  .rbbar { flex: 0 0 auto; height: 30px; display: flex; align-items: center; gap: 10px; padding: 0 14px;
    border-bottom: 1px solid var(--border); font-size: 12px; color: var(--text2); }
  .rbbar .t { flex: 1; }
  .runall { border: 1px solid var(--border); background: transparent; color: var(--accent); font-size: 11.5px;
    padding: 3px 10px; border-radius: 6px; cursor: default; }
  .runall:hover { background: var(--sel); color: var(--text); }
  .rbody { flex: 1; min-height: 0; overflow-y: auto; padding: 16px 22px; max-width: 860px; margin: 0 auto; width: 100%; box-sizing: border-box; }
  .step { display: flex; gap: 10px; align-items: stretch; margin: 10px 0; }
  .step .run { flex: 0 0 auto; align-self: flex-start; border: 1px solid var(--border); background: var(--panel);
    color: var(--accent); font-family: var(--font-ui); font-size: 11.5px; padding: 5px 10px; border-radius: 7px; cursor: default; }
  .step.done .run { color: var(--green); border-color: color-mix(in srgb, var(--green) 50%, var(--border)); }
  .cmd { flex: 1; margin: 0; background: var(--panel); border: 1px solid var(--border); border-radius: 8px;
    padding: 9px 12px; font-family: var(--font-mono); font-size: 12.5px; overflow-x: auto; white-space: pre; }
  .cmd.plain { color: var(--text2); }
  .prose { color: var(--text); font-size: 14px; line-height: 1.6; }
  .prose :global(h1), .prose :global(h2), .prose :global(h3) { color: var(--text); margin: 1.2em 0 .4em; }
  .prose :global(code) { font-family: var(--font-mono); background: var(--panel2); padding: 1px 5px; border-radius: 4px; font-size: .88em; }
  .prose :global(a) { color: var(--accent); }
  .empty { color: var(--text3); padding: 20px; }
</style>
