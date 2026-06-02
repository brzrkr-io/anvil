<script lang="ts">
  // #99 Keymap cheatsheet — reference of every global shortcut, grouped.
  // Opened from the palette ("Help: Keyboard Shortcuts") or ⌘/.
  let { onClose }: { onClose: () => void } = $props();
  let q = $state("");
  const groups: { title: string; keys: [string, string][] }[] = [
    { title: "General", keys: [["⌘K", "Command palette"], ["⌘P", "Find file"], ["⌘,", "Settings"], ["⌘/", "Keyboard shortcuts"], ["⌘+ / ⌘− / ⌘0", "Zoom in / out / reset"]] },
    { title: "Files & Editor", keys: [["⌘O", "Open file"], ["⌘⇧O", "Open folder / Go to symbol"], ["⌘S", "Save"], ["⌘W", "Close tab"], ["⌘⇧T", "Reopen closed tab"], ["⌘E", "Recent files"], ["⌘F", "Find / replace"], ["⌥B", "Toggle blame"], ["⌘⌥K", "Toggle bookmark"], ["⌘⌥J", "Next bookmark"], ["⌘⌥← / →", "Navigate back / forward"], ["F12", "Go to definition"], ["F2", "Rename symbol"], ["⇧F12", "Find references"], ["⌘.", "Code actions"], ["⇧⌥F", "Format document"]] },
    { title: "Terminal", keys: [["⌘T", "New terminal"], ["⌘D", "Split / unsplit"], ["⌘J", "Toggle bottom terminal"]] },
    { title: "Workspace", keys: [["⌘B", "Toggle explorer sidebar"], ["⌘⇧B", "Toggle activity rail"], ["⌘\\", "Split workspace panes"], ["⌘⇧⏎", "Zoom pane"], ["⌘N", "New window"]] },
    { title: "Markdown & Agent", keys: [["⌘⇧V", "Toggle preview"], ["⌘I", "Ask agent"], ["⌘⌥E", "Explain selection"]] },
  ];
  let filtered = $derived(
    q.trim()
      ? groups.map((g) => ({ ...g, keys: g.keys.filter(([k, d]) => (k + " " + d).toLowerCase().includes(q.toLowerCase())) })).filter((g) => g.keys.length)
      : groups,
  );
</script>

<div class="km-scrim" role="presentation" onclick={onClose}>
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="km" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts" tabindex="-1" onclick={(e) => e.stopPropagation()}>
    <header class="km-head">
      <h2>Keyboard shortcuts</h2>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="km-q" placeholder="Filter…" bind:value={q} autofocus />
    </header>
    <div class="km-body">
      {#each filtered as group}
        <section>
          <h3>{group.title}</h3>
          {#each group.keys as [k, d]}
            <div class="km-row"><kbd>{k}</kbd><span>{d}</span></div>
          {/each}
        </section>
      {/each}
      {#if !filtered.length}<p class="km-empty">No shortcuts match.</p>{/if}
    </div>
  </div>
</div>

<style>
  .km-scrim { position: fixed; inset: 0; background: var(--glass-scrim); backdrop-filter: blur(6px); -webkit-backdrop-filter: blur(6px); display: flex; align-items: flex-start; justify-content: center; padding-top: 8vh; z-index: 200; }
  .km { width: 640px; max-width: 94vw; max-height: 78vh; display: flex; flex-direction: column; background: var(--glass); backdrop-filter: blur(var(--blur)) saturate(1.3); -webkit-backdrop-filter: blur(var(--blur)) saturate(1.3); border: 1px solid var(--border); border-radius: 8px; box-shadow: var(--elev-3), inset 0 1px 0 var(--hairline); overflow: hidden; }
  .km-head { display: flex; align-items: center; gap: 12px; padding: 14px 18px; border-bottom: 1px solid var(--border); }
  .km-head h2 { margin: 0; font-size: 15px; font-weight: 600; color: var(--text); white-space: nowrap; }
  .km-q { flex: 1; background: var(--bg); border: 1px solid var(--border); border-radius: 7px; padding: 5px 10px; color: var(--text); font-size: 13px; outline: none; }
  .km-q:focus { border-color: var(--accent); }
  .km-body { overflow-y: auto; padding: 12px 18px 16px; column-count: 2; column-gap: 26px; }
  .km-body section { break-inside: avoid; margin-bottom: 14px; }
  .km-body h3 { margin: 0 0 6px; font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text3); }
  .km-row { display: flex; align-items: baseline; gap: 10px; margin-bottom: 4px; }
  .km-row kbd { flex-shrink: 0; min-width: 78px; font-family: var(--font-mono); font-size: 11px; color: var(--text); background: var(--panel2); border: 1px solid var(--border); border-radius: 5px; padding: 1px 6px; text-align: center; }
  .km-row span { font-size: 12.5px; color: var(--text2); }
  .km-empty { color: var(--text3); font-size: 13px; text-align: center; padding: 20px; }
</style>
