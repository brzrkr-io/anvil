<script lang="ts">
  // Unified stroke icon set (Lucide-style, 24px grid, currentColor). One place
  // to keep glyphs consistent across the app.
  let { name, size = 16, sw = 1.75 }: { name: string; size?: number; sw?: number } = $props();

  const ICONS: Record<string, string> = {
    terminal: '<path d="M5 7l5 5-5 5"/><path d="M13 17h6"/>',
    folder: '<path d="M3 7a2 2 0 0 1 2-2h3.5l2 2H19a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
    branch: '<circle cx="6" cy="6" r="2.4"/><circle cx="6" cy="18" r="2.4"/><circle cx="18" cy="7" r="2.4"/><path d="M6 8.4v7.2"/><path d="M18 9.4A6 6 0 0 1 12 15.4H9"/>',
    search: '<circle cx="11" cy="11" r="7"/><path d="M21 21l-4.3-4.3"/>',
    agent: '<circle cx="12" cy="12" r="4.5"/><path d="M12 3v3"/><path d="M12 18v3"/><path d="M3 12h3"/><path d="M18 12h3"/>',
    basin: '<circle cx="12" cy="12" r="10"/><path d="M2 12 L22 12 A10 10 0 0 1 2 12 Z" fill="currentColor" stroke="none"/>',
    pin: '<path d="M12 3v7"/><path d="M8 10h8l-1 4H9z"/><path d="M12 14v7"/>',
    devops: '<path d="M21 8.5l-9-5-9 5 9 5 9-5z"/><path d="M3 8.5v7l9 5 9-5v-7"/><path d="M12 13.5v7"/>',
    kube: '<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="2.4"/><path d="M12 3v3.6"/><path d="M12 17.4V21"/><path d="M3 12h3.6"/><path d="M17.4 12H21"/><path d="M5.6 5.6l2.6 2.6"/><path d="M15.8 15.8l2.6 2.6"/><path d="M18.4 5.6l-2.6 2.6"/><path d="M8.2 15.8l-2.6 2.6"/>',
    terraform: '<rect x="3.5" y="3.5" width="7" height="7" rx="1"/><rect x="13.5" y="3.5" width="7" height="7" rx="1"/><rect x="3.5" y="13.5" width="7" height="7" rx="1"/>',
    helm: '<circle cx="12" cy="12" r="3.2"/><circle cx="12" cy="12" r="8"/><path d="M12 4v3.2"/><path d="M12 16.8V20"/><path d="M4 12h3.2"/><path d="M16.8 12H20"/>',
    caldera: '<path d="M12 2.5l9.5 9.5L12 21.5 2.5 12z"/><path d="M12 8l4 4-4 4-4-4z"/>',
    workspace: '<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 9.5h18"/><path d="M9.5 21V9.5"/>',
    settings: '<path d="M4 6h8"/><path d="M16 6h4"/><circle cx="14" cy="6" r="2"/><path d="M4 12h2"/><path d="M10 12h10"/><circle cx="8" cy="12" r="2"/><path d="M4 18h10"/><path d="M18 18h2"/><circle cx="16" cy="18" r="2"/>',
    theme: '<circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 0 0 18z" fill="currentColor" stroke="none"/>',
    zoom: '<path d="M21 3h-6"/><path d="M21 3v6"/><path d="M21 3l-7 7"/><path d="M3 21h6"/><path d="M3 21v-6"/><path d="M3 21l7-7"/>',
    ci: '<path d="M3 12a9 9 0 0 1 15-6.7L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-15 6.7L3 16"/><path d="M3 21v-5h5"/>',
    pr: '<circle cx="6" cy="6" r="2.4"/><circle cx="6" cy="18" r="2.4"/><circle cx="18" cy="18" r="2.4"/><path d="M6 8.4v7.2"/><path d="M18 15.6V11a4 4 0 0 0-4-4h-3"/><path d="M13 4l-2 3 2 3"/>',
    chart: '<path d="M3 3v18h18"/><path d="M7 15l3-4 3 3 4-6"/>',
    plus: '<path d="M12 5v14"/><path d="M5 12h14"/>',
    close: '<path d="M6 6l12 12"/><path d="M18 6L6 18"/>',
    play: '<path d="M7 4l12 8-12 8z"/>',
    refresh: '<path d="M3 12a9 9 0 0 1 15-6.7L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-15 6.7L3 16"/><path d="M3 21v-5h5"/>',
    minus: '<path d="M5 12h14"/>',
    check: '<path d="M20 6L9 17l-5-5"/>',
    tag: '<path d="M3 11.5V4a1 1 0 0 1 1-1h7.5a1 1 0 0 1 .7.3l8 8a1 1 0 0 1 0 1.4l-7.5 7.5a1 1 0 0 1-1.4 0l-8-8a1 1 0 0 1-.3-.7z"/><circle cx="7.5" cy="7.5" r="1.3"/>',
    stash: '<path d="M21 8v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V8"/><path d="M2 4h20v4H2z"/><path d="M9 12h6"/>',
    paperclip: '<path d="M21 9l-9.5 9.5a4.5 4.5 0 0 1-6.4-6.4L14 3.2a3 3 0 0 1 4.2 4.2L9 16.5a1.5 1.5 0 0 1-2.1-2.1L15 6.3"/>',
    pencil: '<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/>',
    up: '<path d="M12 19V5"/><path d="M5 12l7-7 7 7"/>',
    density: '<path d="M3 8V5a2 2 0 0 1 2-2h3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M21 16v3a2 2 0 0 1-2 2h-3"/>',
    command: '<path d="M9 6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3z"/>',
    history: '<path d="M3 3v5h5"/><path d="M3.05 13A9 9 0 1 0 6 5.3L3 8"/><path d="M12 7v5l4 2"/>',
    key: '<circle cx="7.5" cy="15.5" r="3.5"/><path d="M10 13L20 3"/><path d="M16 7l2.5 2.5"/><path d="M14 9l2.5 2.5"/>',
    info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5"/><path d="M12 8h.01"/>',
    alert: '<path d="M10.3 3.8 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.8a2 2 0 0 0-3.4 0z"/><path d="M12 9v4"/><path d="M12 17h.01"/>',
    globe: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18"/><path d="M12 3a14 14 0 0 1 0 18a14 14 0 0 1 0-18z"/>',
  };
</script>

<svg class="ico" width={size} height={size} viewBox="0 0 24 24" fill="none"
  stroke="currentColor" stroke-width={sw} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
>{@html ICONS[name] ?? ""}</svg>

<style>
  .ico { display: block; flex: 0 0 auto; }
</style>
