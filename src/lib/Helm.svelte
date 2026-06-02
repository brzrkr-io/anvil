<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import { readCache, writeCache } from "$lib/cache";
  import { toast } from "$lib/toast";
  import CodeView from "$lib/CodeView.svelte";

  interface Release {
    name: string; namespace: string; revision: string;
    updated: string; status: string; chart: string; app_version: string;
  }
  type Detail = "values" | "all" | "history" | "status" | "manifest";

  let releases = $state<Release[]>([]);
  let selected = $state<Release | null>(null);
  let detail = $state<Detail>("values");
  let content = $state("");
  let history = $state<{ revision: number; updated: string; status: string; chart: string; description: string }[]>([]);
  let loading = $state(false);
  let err = $state("");

  async function load() {
    loading = true; err = "";
    try {
      const raw = await invoke<string>("helm_list");
      releases = (JSON.parse(raw || "[]") as Release[]).sort(
        (a, b) => a.namespace.localeCompare(b.namespace) || a.name.localeCompare(b.name),
      );
      writeCache("helm-releases", releases);
    } catch (e) {
      err = String(e);
    } finally {
      loading = false;
    }
  }

  async function loadDetail() {
    if (!selected) return;
    content = "Loading…"; history = [];
    const { name, namespace } = selected;
    try {
      if (detail === "history") {
        const raw = await invoke<string>("helm_history", { name, namespace });
        history = JSON.parse(raw || "[]");
        content = "";
      } else {
        const cmd = detail === "values" ? "helm_values" : detail === "all" ? "helm_values_all" : detail === "status" ? "helm_status" : "helm_manifest";
        content = await invoke<string>(cmd, { name, namespace });
      }
    } catch (e) {
      content = String(e);
    }
  }

  async function select(r: Release) {
    selected = r; detail = "values"; diffText = ""; diffRevNum = 0;
    await loadDetail();
  }
  function setDetail(d: Detail) { detail = d; diffText = ""; diffRevNum = 0; loadDetail(); }

  let busy = $state("");
  let diffText = $state("");
  let diffRevNum = $state(0);

  async function diffRev(rev: number) {
    if (!selected || busy) return;
    busy = `diff${rev}`; diffText = "Loading diff…"; diffRevNum = rev;
    try {
      diffText = await invoke<string>("helm_diff_revision", { name: selected.name, namespace: selected.namespace, revision: String(rev) });
      if (!diffText.trim()) diffText = "No differences from the deployed revision.";
    } catch (e) {
      diffText = `${e}`.includes("unknown command") || `${e}`.includes("diff")
        ? "helm diff failed — the helm-diff plugin may not be installed (`helm plugin install https://github.com/databus23/helm-diff`)."
        : String(e);
    } finally { busy = ""; }
  }

  async function rollback(rev: number) {
    if (!selected || busy) return;
    if (!confirm(`Roll back ${selected.namespace}/${selected.name} to revision v${rev}?\n\nThis re-applies that revision's manifests to the cluster.`)) return;
    busy = `rb${rev}`;
    try {
      const out = await invoke<string>("helm_rollback", { name: selected.name, namespace: selected.namespace, revision: String(rev) });
      toast(`Rolled back ${selected.name} to v${rev}: ${out.trim().split("\n").pop() || "done"}`.slice(0, 140), "success");
      await loadDetail();
    } catch (e) {
      toast(String(e).slice(0, 160), "error");
    } finally { busy = ""; }
  }

  function statusColor(s: string): string {
    if (s === "deployed") return "var(--green)";
    if (s === "failed") return "var(--red)";
    if (s === "pending-install" || s === "pending-upgrade" || s === "pending-rollback") return "var(--accent)";
    if (s === "superseded" || s === "uninstalled") return "var(--text3)";
    return "var(--yellow)";
  }

  interface NsGroup { ns: string; items: Release[]; }
  const groups = $derived.by<NsGroup[]>(() => {
    const m = new Map<string, Release[]>();
    for (const r of releases) {
      const arr = m.get(r.namespace) ?? [];
      arr.push(r);
      m.set(r.namespace, arr);
    }
    return [...m.entries()].map(([ns, items]) => ({ ns, items }));
  });

  // Show last-known releases instantly, then refresh in the background.
  onMount(() => { releases = readCache<Release[]>("helm-releases") ?? releases; load(); });
</script>

<div class="helm">
  <div class="topbar">
    <span class="lbl">Helm releases</span>
    <span class="count">{releases.length}</span>
    <span class="spacer"></span>
    <button class="iconbtn" onclick={load} title="Refresh"><Icon name="refresh" size={13} /></button>
  </div>

  {#if err}
    <div class="err">{err.includes("helm") && err.includes("not") ? "helm not found in PATH." : err.slice(0, 200)}</div>
  {/if}

  <div class="body">
    <div class="list">
      {#if loading && !releases.length}
        <div class="empty">Loading…</div>
      {:else if !releases.length && !err}
        <div class="empty">No Helm releases found.</div>
      {:else}
        {#each groups as g (g.ns)}
          <div class="ns">{g.ns}</div>
          {#each g.items as r (r.namespace + r.name)}
            <div class="rel" class:on={selected?.namespace === r.namespace && selected?.name === r.name}
              role="button" tabindex="0" onclick={() => select(r)} onkeydown={(e) => e.key === "Enter" && select(r)}>
              <span class="dot" style="background:{statusColor(r.status)}"></span>
              <span class="rname">{r.name}</span>
              <span class="rchart">{r.chart}</span>
              <span class="rrev">v{r.revision}</span>
            </div>
          {/each}
        {/each}
      {/if}
    </div>

    {#if selected}
      <div class="detail">
        <div class="dhead">
          <span class="dtitle">{selected.name}</span>
          <span class="dstatus" style="color:{statusColor(selected.status)}">{selected.status}</span>
          <span class="dmeta">{selected.chart} · {selected.app_version || "—"}</span>
          <span class="spacer"></span>
          <button class="iconbtn" onclick={() => (selected = null)} title="Close"><Icon name="close" size={13} /></button>
        </div>
        <div class="tabs">
          {#each [["values", "Overrides"], ["all", "All values"], ["history", "History"], ["status", "Status"], ["manifest", "Manifest"]] as const as [d, label]}
            <button class="tab" class:on={detail === d} onclick={() => setDetail(d)}>{label}</button>
          {/each}
        </div>
        {#if detail === "history"}
          <div class="hist">
            <div class="h-head"><span>Rev</span><span>Status</span><span>Chart</span><span>Description</span><span></span></div>
            {#each history as h (h.revision)}
              <div class="h-row" class:cur={h.revision === Math.max(...history.map((x) => x.revision))}>
                <span class="h-rev">v{h.revision}</span>
                <span class="h-st" style="color:{statusColor(h.status)}">{h.status}</span>
                <span class="h-chart">{h.chart}</span>
                <span class="h-desc">{h.description}</span>
                <span class="h-acts">
                  <button class="h-act" title="Diff this revision vs deployed" disabled={!!busy} onclick={() => diffRev(h.revision)}>diff</button>
                  {#if h.revision !== Math.max(...history.map((x) => x.revision))}
                    <button class="h-act warn" title="Roll back to this revision" disabled={!!busy} onclick={() => rollback(h.revision)}>rollback</button>
                  {/if}
                </span>
              </div>
            {/each}
            {#if !history.length}<div class="empty">No history.</div>{/if}
            {#if diffText}
              <div class="h-diff-head">Diff — v{diffRevNum} vs deployed <button class="h-x" onclick={() => { diffText = ""; }} title="Close">×</button></div>
              <pre class="out diff">{diffText}</pre>
            {/if}
          </div>
        {:else}
          <div class="out cv"><CodeView text={content} lang="yaml" /></div>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .helm { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .topbar {
    display: flex; align-items: center; gap: 8px; height: 30px; flex: 0 0 auto;
    padding: 0 12px; border-bottom: 1px solid var(--border);
  }
  .lbl { color: var(--text3); font-size: 11px; font-weight: 500; }
  .count { font-family: var(--font-mono); font-size: 9.5px; color: var(--text3); opacity: 0.7; }
  .spacer { flex: 1; }
  .iconbtn {
    display: inline-flex; align-items: center; justify-content: center;
    width: 22px; height: 20px; border: 0; border-radius: 5px; background: transparent; color: var(--text3); cursor: default;
  }
  .iconbtn:hover { background: var(--sel); color: var(--text); }
  .err { padding: 8px 12px; font-size: 11.5px; color: var(--red); border-bottom: 1px solid var(--border); font-family: var(--font-mono); }
  .empty { padding: 18px 14px; color: var(--text3); font-size: 12px; }

  .body { flex: 1; min-height: 0; display: flex; overflow: hidden; }
  .list { flex: 0 0 320px; min-width: 240px; overflow-y: auto; border-right: 1px solid var(--border); }
  .ns {
    padding: 5px 12px 3px; font-size: 10px; font-weight: 500; color: var(--text3);
    text-transform: uppercase; letter-spacing: 0.05em; background: var(--panel);
    border-bottom: 1px solid var(--hairline); position: sticky; top: 0; z-index: 1;
  }
  .rel {
    display: grid; grid-template-columns: 14px minmax(0,1fr) auto 32px; align-items: center; column-gap: 8px;
    height: 24px; padding: 0 12px; font-size: 11.5px; cursor: default; border-bottom: 1px solid var(--hairline);
  }
  .rel:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }
  .rel.on { background: var(--sel); }
  .dot { width: 7px; height: 7px; border-radius: 50%; }
  .rname { color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rchart { color: var(--text3); font-family: var(--font-mono); font-size: 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rrev { font-family: var(--font-mono); font-size: 10px; color: var(--text3); text-align: right; }

  .detail { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .dhead {
    display: flex; align-items: center; gap: 8px; height: 30px; flex: 0 0 auto;
    padding: 0 12px; border-bottom: 1px solid var(--border); font-size: 11.5px;
  }
  .dtitle { color: var(--text); font-weight: 500; }
  .dstatus { font-size: 11px; }
  .dmeta { color: var(--text3); font-family: var(--font-mono); font-size: 10px; }
  .tabs { display: flex; gap: 2px; padding: 4px 8px; border-bottom: 1px solid var(--border); flex: 0 0 auto; }
  .tab {
    border: 0; background: transparent; color: var(--text3); cursor: default;
    font-size: 11px; padding: 3px 9px; border-radius: 5px;
  }
  .tab:hover { color: var(--text2); }
  .tab.on { background: var(--sel); color: var(--text); }
  .out {
    flex: 1; min-height: 0; overflow: auto; margin: 0; padding: 10px 12px; background: var(--bg);
    font-family: var(--font-mono); font-size: 11px; line-height: 1.5; color: var(--text2); white-space: pre;
  }
  .hist { flex: 1; min-height: 0; overflow: auto; }
  .h-head, .h-row {
    display: grid; grid-template-columns: 40px 90px 150px minmax(0,1fr) auto; column-gap: 10px;
    align-items: center; padding: 0 12px; height: 26px; border-bottom: 1px solid var(--hairline); font-size: 11px;
  }
  .h-head { color: var(--text3); font-size: 10px; text-transform: uppercase; letter-spacing: 0.04em; position: sticky; top: 0; background: var(--panel); }
  .h-row.cur { background: color-mix(in srgb, var(--accent) 7%, transparent); }
  .h-rev { font-family: var(--font-mono); }
  .h-chart { font-family: var(--font-mono); font-size: 10px; color: var(--text3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .h-desc { color: var(--text2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .h-acts { display: flex; gap: 4px; justify-content: flex-end; }
  .h-act { border: 1px solid var(--border); background: transparent; color: var(--text2);
    font-family: var(--font-ui); font-size: 10px; height: 18px; padding: 0 7px; border-radius: 4px; cursor: default; }
  .h-act:hover:not(:disabled) { color: var(--text); border-color: var(--text3); }
  .h-act.warn:hover:not(:disabled) { color: var(--accent2); border-color: var(--accent2); }
  .h-act:disabled { opacity: 0.45; }
  .h-diff-head { display: flex; align-items: center; gap: 8px; padding: 6px 12px 4px; font-size: 11px; color: var(--text2); font-weight: 600; }
  .h-x { margin-left: auto; border: 0; background: transparent; color: var(--text3); cursor: default; font-size: 14px; line-height: 1; }
  .h-x:hover { color: var(--text); }
  .out.diff { margin: 0; }
  /* CodeView brings its own scroller, font, and padding — strip the <pre> styling. */
  .out.cv { padding: 0; overflow: hidden; white-space: normal; }
</style>
