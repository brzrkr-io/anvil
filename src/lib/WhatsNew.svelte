<script lang="ts">
  // #97 In-app changelog / what's-new. Auto-shows once per release (gated by
  // the `anvil-seen-version` localStorage key in +page) and is reopenable from
  // the command palette ("Help: What's New").
  let { version, notes, onClose }: { version: string; notes: { title: string; items: string[] }[]; onClose: () => void } = $props();
</script>

<div class="wn-scrim" role="presentation" onclick={onClose}>
  <div class="wn" role="dialog" aria-modal="true" aria-label="What's new" onclick={(e) => e.stopPropagation()}>
    <header class="wn-head">
      <span class="wn-badge">v{version}</span>
      <h2>What's new</h2>
    </header>
    <div class="wn-body">
      {#each notes as group}
        <section>
          <h3>{group.title}</h3>
          <ul>
            {#each group.items as item}<li>{item}</li>{/each}
          </ul>
        </section>
      {/each}
    </div>
    <button class="wn-go" onclick={onClose}>Got it</button>
  </div>
</div>

<style>
  .wn-scrim { position: fixed; inset: 0; background: var(--glass-scrim); backdrop-filter: blur(6px); -webkit-backdrop-filter: blur(6px); display: flex; align-items: center; justify-content: center; z-index: 200; }
  .wn { width: 560px; max-width: 92vw; max-height: 80vh; display: flex; flex-direction: column; background: var(--glass); backdrop-filter: blur(var(--blur)) saturate(1.3); -webkit-backdrop-filter: blur(var(--blur)) saturate(1.3); border: 1px solid var(--border); border-radius: 8px; box-shadow: var(--elev-3), inset 0 1px 0 var(--hairline); overflow: hidden; }
  .wn-head { display: flex; align-items: center; gap: 10px; padding: 18px 22px 10px; }
  .wn-head h2 { margin: 0; font-size: 18px; font-weight: 600; color: var(--text); }
  .wn-badge { font-size: 11px; font-weight: 600; color: var(--bg); background: var(--accent); border-radius: 6px; padding: 2px 8px; }
  .wn-body { overflow-y: auto; padding: 4px 22px 8px; }
  .wn-body section { margin-bottom: 14px; }
  .wn-body h3 { margin: 0 0 6px; font-size: 12px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text3); }
  .wn-body ul { margin: 0; padding-left: 18px; }
  .wn-body li { font-size: 13px; line-height: 1.55; color: var(--text2); margin-bottom: 3px; }
  .wn-go { margin: 8px 22px 18px; align-self: flex-end; padding: 7px 18px; font-size: 13px; font-weight: 500; color: var(--bg); background: var(--accent); border: none; border-radius: 8px; cursor: pointer; }
  .wn-go:hover { filter: brightness(1.08); }
</style>
