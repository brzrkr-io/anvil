<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  // Centered, calm empty state — an icon + line + optional hint — instead of a
  // bare line of text floating in a blank pane. (#96)
  let { icon = "info", title, hint = "" }: { icon?: string; title: string; hint?: string } = $props();
</script>

<div class="empty-state">
  <span class="es-icon"><Icon name={icon} size={26} /></span>
  <span class="es-title">{title}</span>
  {#if hint}<span class="es-hint">{hint}</span>{/if}
</div>

<style>
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 40px 16px;
    color: var(--text3);
    text-align: center;
  }
  .es-icon {
    display: flex;
    opacity: 0.5;
  }
  .es-title {
    font-size: 13px;
    color: var(--text2);
  }
  .es-hint {
    font-size: 11.5px;
    color: var(--text3);
    max-width: 320px;
    line-height: 1.5;
  }
</style>
