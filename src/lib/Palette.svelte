<script lang="ts" module>
  export type Item = { label: string; hint?: string; run: () => void };
</script>

<script lang="ts">
  import { rank } from "$lib/fuzzy";

  let {
    open = $bindable(false),
    items = [],
    placeholder = "",
  }: { open: boolean; items: Item[]; placeholder?: string } = $props();

  let query = $state("");
  let sel = $state(0);
  let inputEl = $state<HTMLInputElement>();

  // Recents (#82): recently-run command labels float to the top on empty query.
  const RECENTS_KEY = "anvil-cmd-recents";
  function loadRecents(): string[] {
    if (typeof localStorage === "undefined") return [];
    try { return JSON.parse(localStorage.getItem(RECENTS_KEY) || "[]"); } catch { return []; }
  }
  let recents = $state<string[]>(loadRecents());

  const filtered = $derived.by(() => {
    if (query.trim()) return rank(items, query, (i) => i.label, 300);
    // Empty query: recents first (in recency order), then the rest as given.
    const order = new Map(recents.map((l, i) => [l, i]));
    return [...items].sort((a, b) => {
      const ra = order.has(a.label) ? order.get(a.label)! : Infinity;
      const rb = order.has(b.label) ? order.get(b.label)! : Infinity;
      return ra - rb;
    });
  });

  $effect(() => {
    if (open) {
      query = "";
      sel = 0;
      queueMicrotask(() => inputEl?.focus());
    }
  });
  $effect(() => {
    if (sel >= filtered.length) sel = Math.max(0, filtered.length - 1);
  });

  function choose(it: Item) {
    open = false;
    recents = [it.label, ...recents.filter((l) => l !== it.label)].slice(0, 12);
    if (typeof localStorage !== "undefined") { try { localStorage.setItem(RECENTS_KEY, JSON.stringify(recents)); } catch { /* ignore */ } }
    it.run();
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") { open = false; e.preventDefault(); }
    else if (e.key === "ArrowDown") { sel = Math.min(sel + 1, filtered.length - 1); e.preventDefault(); }
    else if (e.key === "ArrowUp") { sel = Math.max(sel - 1, 0); e.preventDefault(); }
    else if (e.key === "Enter") { const it = filtered[sel]; if (it) choose(it); e.preventDefault(); }
  }
</script>

{#if open}
  <div class="scrim" onclick={() => (open = false)} role="presentation"></div>
  <div class="palette" role="dialog" aria-modal="true">
    <input bind:this={inputEl} bind:value={query} {placeholder} onkeydown={onKey} spellcheck="false" />
    <div class="list">
      {#each filtered as it, i (it.label + i)}
        <div class="pi {i === sel ? 'sel' : ''}" onclick={() => choose(it)} onkeydown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), choose(it))} onmouseenter={() => (sel = i)} role="button" tabindex="-1">
          <span class="lbl">{it.label}</span>
          {#if it.hint}<span class="hint">{it.hint}</span>{/if}
        </div>
      {/each}
      {#if filtered.length === 0}<div class="pi empty">No matches</div>{/if}
    </div>
  </div>
{/if}

<style>
  .scrim { position: fixed; inset: 0; background: var(--glass-scrim); backdrop-filter: blur(2px); -webkit-backdrop-filter: blur(2px); z-index: 90; }
  .palette {
    position: fixed; top: 64px; left: 50%; transform: translateX(-50%);
    width: min(620px, 86vw); z-index: 91;
    background: var(--panel); border: 1px solid var(--border); border-radius: 6px;
    box-shadow: var(--elev-3); overflow: hidden;
    animation: pal-in 0.14s cubic-bezier(0.2, 0.9, 0.3, 1);
  }
  @keyframes pal-in { from { opacity: 0; transform: translate(-50%, -6px); } to { opacity: 1; transform: translate(-50%, 0); } }
  input {
    width: 100%; padding: 13px 16px; border: 0; outline: 0;
    background: transparent; color: var(--text); font-size: 14px; font-family: var(--font-ui);
    border-bottom: 1px solid var(--hairline);
  }
  .list { max-height: 50vh; overflow-y: auto; padding: 6px; }
  .pi {
    display: flex; align-items: center; gap: 10px; padding: 7px 11px; border-radius: 8px;
    font-size: 13px; color: var(--text2); cursor: default; transition: background 0.08s ease;
  }
  .pi.sel { background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--text); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 35%, transparent); }
  .lbl { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .hint { color: var(--text3); font-size: 11px; font-family: var(--font-mono); }
  .empty { color: var(--text3); }
</style>
