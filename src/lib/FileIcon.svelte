<script lang="ts">
  // Real folder + document SVG icons, tinted by file type. Replaces the old
  // square/letter badges.
  let { name = "", dir = false, open = false }: { name?: string; dir?: boolean; open?: boolean } = $props();

  const COLOR: Record<string, string> = {
    ts: "#3178c6", tsx: "#3178c6", js: "#e8c14e", jsx: "#e8c14e", mjs: "#e8c14e",
    json: "#e8c14e", rs: "#e06a3b", py: "#4b8bbe", go: "#46c3d6", rb: "#e0556f",
    svelte: "#ff5722", vue: "#42b883", html: "#e06a3b", css: "#42a5f5", scss: "#cf649a",
    md: "#9aa4b8", sh: "#9bc777", bash: "#9bc777", zsh: "#9bc777", fish: "#9bc777",
    toml: "#9aa4b8", yaml: "#cb4b16", yml: "#cb4b16", lock: "#8a7f73", ini: "#9aa4b8",
    conf: "#9aa4b8", env: "#9bc777", dockerfile: "#2496ed", makefile: "#9bc777",
    png: "#a371f7", jpg: "#a371f7", jpeg: "#a371f7", svg: "#42a5f5", gif: "#a371f7", webp: "#a371f7",
    pdf: "#e0556f", zip: "#c9a227", tar: "#c9a227", gz: "#c9a227",
    sql: "#d98b3a", c: "#5f8fd6", h: "#5f8fd6", cpp: "#5f8fd6", java: "#e0772b", txt: "#9aa4b8",
  };

  function key(n: string): string {
    const lc = n.toLowerCase();
    if (lc === "dockerfile") return "dockerfile";
    if (lc === "makefile") return "makefile";
    return lc.split(".").pop() ?? "";
  }
  const color = $derived(dir ? "var(--accent)" : (COLOR[key(name)] ?? "var(--text3)"));
</script>

{#if dir}
  <!-- Two-tone folder: lighter back panel + tab, solid front pocket. -->
  <svg class="fi" viewBox="0 0 16 16" fill="none" aria-hidden="true">
    <path d="M1.5 4.4a1 1 0 0 1 1-1H6l1.4 1.6h6.1a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1H2.5a1 1 0 0 1-1-1Z"
      fill={color} opacity="0.4" />
    {#if open}
      <path d="M3.4 6.7a1 1 0 0 1 .96-.74h10.1a.72.72 0 0 1 .69.92l-1.45 4.9a1 1 0 0 1-.96.72H1.7a.6.6 0 0 1-.58-.79Z"
        fill={color} />
    {:else}
      <path d="M1.5 6.6a1 1 0 0 1 1-1h11a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1H2.5a1 1 0 0 1-1-1Z"
        fill={color} />
    {/if}
  </svg>
{:else}
  <svg class="fi" viewBox="0 0 16 16" fill="none" aria-hidden="true">
    <path d="M4 1.6h5l3.4 3.4V13a1.2 1.2 0 0 1-1.2 1.2H4A1.2 1.2 0 0 1 2.8 13V2.8A1.2 1.2 0 0 1 4 1.6Z"
      fill={color} opacity="0.18" stroke={color} stroke-width="1" stroke-linejoin="round" />
    <path d="M9 1.8V5h3.2" stroke={color} stroke-width="1" stroke-linejoin="round" />
  </svg>
{/if}

<style>
  .fi { width: 15px; height: 15px; flex: 0 0 auto; display: block; }
</style>
