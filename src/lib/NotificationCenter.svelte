<script lang="ts">
  import { notifications, markAllRead, clearNotifications } from "$lib/toast";
  import Icon from "$lib/Icon.svelte";

  let { open = $bindable(false) }: { open?: boolean } = $props();

  // Mark everything read whenever the panel is shown.
  $effect(() => {
    if (open) markAllRead();
  });

  function rel(ts: number): string {
    const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
    if (s < 60) return `${s}s`;
    const m = Math.round(s / 60);
    if (m < 60) return `${m}m`;
    const h = Math.round(m / 60);
    if (h < 24) return `${h}h`;
    return `${Math.round(h / 24)}d`;
  }
</script>

{#if open}
  <div class="nc-scrim" role="presentation" onclick={() => (open = false)}></div>
  <div class="nc" role="dialog" aria-label="Notifications">
    <div class="nc-head">
      <span class="nc-title">Notifications</span>
      <span class="spacer"></span>
      <button class="nc-act" onclick={clearNotifications} disabled={$notifications.length === 0}>Clear</button>
      <button class="nc-x" onclick={() => (open = false)} title="Close"><Icon name="close" size={12} /></button>
    </div>
    {#if $notifications.length === 0}
      <div class="nc-empty">No notifications</div>
    {:else}
      <ul class="nc-list">
        {#each $notifications as n, i (n.id + '#' + i)}
          <li class="nc-item {n.kind}">
            <span class="nc-dot"></span>
            <span class="nc-text">{n.text}</span>
            <span class="nc-time">{rel(n.ts)}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .nc-scrim {
    position: fixed;
    inset: 0;
    z-index: 60;
  }
  .nc {
    position: fixed;
    right: 8px;
    bottom: 34px;
    z-index: 61;
    width: 320px;
    max-height: 60vh;
    display: flex;
    flex-direction: column;
    background: var(--bg1);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.4);
    overflow: hidden;
  }
  .nc-head {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 8px 0 10px;
    border-bottom: 1px solid var(--border);
  }
  .nc-title {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text2);
  }
  .spacer {
    flex: 1;
  }
  .nc-act {
    border: 0;
    background: transparent;
    color: var(--text3);
    font-size: 11px;
    cursor: default;
    padding: 2px 6px;
    border-radius: 4px;
  }
  .nc-act:hover:not(:disabled) {
    background: var(--sel);
    color: var(--text);
  }
  .nc-act:disabled {
    opacity: 0.4;
  }
  .nc-x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--text3);
    cursor: default;
  }
  .nc-x:hover {
    background: var(--sel);
    color: var(--text);
  }
  .nc-empty {
    padding: 24px 12px;
    text-align: center;
    color: var(--text3);
    font-size: 12px;
  }
  .nc-list {
    list-style: none;
    margin: 0;
    padding: 4px 0;
    overflow-y: auto;
  }
  .nc-item {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 6px 10px;
    font-size: 12px;
  }
  .nc-dot {
    flex: 0 0 auto;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text3);
    transform: translateY(-1px);
  }
  .nc-item.success .nc-dot {
    background: var(--green);
  }
  .nc-item.error .nc-dot {
    background: var(--red);
  }
  .nc-text {
    flex: 1;
    min-width: 0;
    color: var(--text);
    word-break: break-word;
  }
  .nc-time {
    flex: 0 0 auto;
    color: var(--text3);
    font-variant-numeric: tabular-nums;
  }
</style>
