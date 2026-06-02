<script lang="ts">
  import { problems, type Problem } from "$lib/diagnostics";

  let { onOpen }: { onOpen?: (path: string, line: number) => void } = $props();

  interface Group { path: string; name: string; items: Problem[]; errs: number; warns: number; }

  const groups = $derived.by<Group[]>(() => {
    const m = new Map<string, Problem[]>();
    for (const p of $problems) {
      const arr = m.get(p.path) ?? [];
      arr.push(p);
      m.set(p.path, arr);
    }
    return [...m.entries()]
      .map(([path, items]) => ({
        path,
        name: path.split("/").pop() ?? path,
        items: [...items].sort((a, b) => a.line - b.line),
        errs: items.filter((i) => i.severity === 1).length,
        warns: items.filter((i) => i.severity === 2).length,
      }))
      .sort((a, b) => (b.errs - a.errs) || a.name.localeCompare(b.name));
  });

  const totalErr = $derived($problems.filter((p) => p.severity === 1).length);
  const totalWarn = $derived($problems.filter((p) => p.severity === 2).length);

  function sevClass(s: number): string {
    return s === 1 ? "err" : s === 2 ? "warn" : "info";
  }
  function sevGlyph(s: number): string {
    return s === 1 ? "✕" : s === 2 ? "▲" : "ⓘ";
  }
</script>

<div class="problems">
  <div class="ph">
    <span class="ph-t">Problems</span>
    {#if totalErr}<span class="cnt err">✕ {totalErr}</span>{/if}
    {#if totalWarn}<span class="cnt warn">▲ {totalWarn}</span>{/if}
  </div>
  <div class="body">
    {#if !$problems.length}
      <div class="empty">No problems detected. ✓</div>
    {:else}
      {#each groups as g (g.path)}
        <div class="file" title={g.path}>
          <span class="fname">{g.name}</span>
          <span class="fpath">{g.path.replace(/\/[^/]+$/, "")}</span>
          <span class="fcnt">{g.items.length}</span>
        </div>
        {#each g.items as d (d.line + d.message)}
          <div
            class="prow"
            role="button"
            tabindex="0"
            onclick={() => onOpen?.(g.path, d.line)}
            onkeydown={(e) => e.key === "Enter" && onOpen?.(g.path, d.line)}
          >
            <span class="sev {sevClass(d.severity)}">{sevGlyph(d.severity)}</span>
            <span class="msg">{d.message}</span>
            <span class="ln">{d.line}</span>
          </div>
        {/each}
      {/each}
    {/if}
  </div>
</div>

<style>
  .problems { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .ph {
    display: flex; align-items: center; gap: 8px; height: 24px; flex: 0 0 auto; padding: 0 12px;
    border-bottom: 1px solid var(--border); font-size: 10.5px; font-weight: 500;
    color: var(--text3); text-transform: uppercase; letter-spacing: 0.04em;
  }
  .cnt { font-family: var(--font-mono); font-size: 10px; letter-spacing: 0; text-transform: none; }
  .cnt.err { color: var(--red); }
  .cnt.warn { color: var(--yellow); }
  .body { flex: 1; overflow-y: auto; }
  .empty { padding: 18px 14px; color: var(--text3); font-size: 12px; }

  .file {
    display: flex; align-items: baseline; gap: 8px; height: 22px; padding: 0 12px;
    background: var(--panel); border-bottom: 1px solid var(--hairline);
    position: sticky; top: 0; z-index: 1;
  }
  .fname { font-size: 11.5px; color: var(--text2); font-weight: 500; }
  .fpath { font-size: 10px; color: var(--text3); opacity: 0.6; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
  .fcnt { font-family: var(--font-mono); font-size: 9.5px; color: var(--text3); opacity: 0.7; }

  .prow {
    display: flex; align-items: baseline; gap: 9px; padding: 3px 12px 3px 22px;
    font-size: 11.5px; cursor: default; border-bottom: 1px solid var(--hairline);
  }
  .prow:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }
  .sev { flex: 0 0 auto; font-size: 9px; }
  .sev.err { color: var(--red); }
  .sev.warn { color: var(--yellow); }
  .sev.info { color: var(--text3); }
  .msg { flex: 1; min-width: 0; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ln { flex: 0 0 auto; font-family: var(--font-mono); font-size: 10px; color: var(--text3); }
</style>
