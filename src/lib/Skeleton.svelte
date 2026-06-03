<script lang="ts">
  // Lightweight shimmer placeholder rows shown on first load instead of a blank
  // pane or a bare "Loading…". Pure CSS, no JS cost. (#96)
  let { rows = 8, gap = 6 }: { rows?: number; gap?: number } = $props();
</script>

<div class="skel" style="gap:{gap}px" aria-hidden="true">
  {#each Array(rows) as _, i (i)}
    <div class="skel-row" style="width:{70 + ((i * 37) % 28)}%"></div>
  {/each}
</div>

<style>
  .skel {
    display: flex;
    flex-direction: column;
    padding: 10px 12px;
  }
  .skel-row {
    height: 12px;
    border-radius: 5px;
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--text) 6%, transparent) 25%,
      color-mix(in srgb, var(--text) 12%, transparent) 37%,
      color-mix(in srgb, var(--text) 6%, transparent) 63%
    );
    background-size: 400% 100%;
    animation: skel-shimmer 1.4s ease-in-out infinite;
  }
  @keyframes skel-shimmer {
    0% { background-position: 100% 0; }
    100% { background-position: 0 0; }
  }
  @media (prefers-reduced-motion: reduce) {
    .skel-row { animation: none; }
  }
</style>
