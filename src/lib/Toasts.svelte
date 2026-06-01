<script lang="ts">
  import { fly } from "svelte/transition";
  import { toasts, dismiss } from "$lib/toast";
</script>

<div class="stack">
  {#each $toasts as t (t.id)}
    <div
      class="card"
      role="button"
      tabindex="0"
      onkeydown={(e) => e.key === "Enter" && dismiss(t.id)}
      transition:fly={{ y: 8, duration: 180 }}
      onclick={() => dismiss(t.id)}
    >
      <span class="stripe kind-{t.kind}"></span>
      <span class="msg">{t.text}</span>
    </div>
  {/each}
</div>

<style>
  .stack {
    position: fixed;
    bottom: 34px;
    right: 14px;
    z-index: 80;
    display: flex;
    flex-direction: column;
    gap: 6px;
    align-items: flex-end;
    pointer-events: none;
  }
  .card {
    display: flex;
    align-items: stretch;
    background: var(--panel2);
    border: 1px solid var(--border);
    border-radius: 7px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.35);
    overflow: hidden;
    min-width: 220px;
    max-width: 340px;
    cursor: default;
    pointer-events: auto;
  }
  .stripe {
    width: 4px;
    flex-shrink: 0;
  }
  .kind-info    { background: var(--accent); }
  .kind-success { background: var(--green); }
  .kind-error   { background: var(--red); }
  .msg {
    padding: 8px 12px;
    font-family: var(--font-ui);
    font-size: 12.5px;
    color: var(--text);
    line-height: 1.4;
    word-break: break-word;
  }
</style>
