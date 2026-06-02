<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import { askText } from "$lib/dialog";

  const ls = typeof localStorage !== "undefined" ? localStorage : null;
  let view = $state<"metrics" | "logs">("metrics");

  // ── Prometheus ──
  let promBase = $state(ls?.getItem("anvil-prom-base") ?? "");
  let promQuery = $state("");
  let promRows = $state<{ metric: string; value: string }[]>([]);
  let promErr = $state("");
  let promBusy = $state(false);

  function metricLabel(m: Record<string, string>): string {
    const name = m.__name__ ?? "";
    const labels = Object.entries(m).filter(([k]) => k !== "__name__").map(([k, v]) => `${k}="${v}"`).join(", ");
    return labels ? `${name}{${labels}}` : name || "{}";
  }

  async function runProm() {
    if (!promBase || !promQuery) return;
    ls?.setItem("anvil-prom-base", promBase);
    promErr = ""; promRows = []; promBusy = true;
    try {
      const j = JSON.parse(await invoke<string>("prom_query", { base: promBase, query: promQuery }));
      if (j.status !== "success") { promErr = j.error || "query error"; return; }
      promRows = (j.data?.result ?? []).map((r: any) => ({ metric: metricLabel(r.metric ?? {}), value: r.value?.[1] ?? "" }));
      if (!promRows.length) promErr = "no matching series";
    } catch (e) { promErr = String(e); } finally { promBusy = false; }
  }

  // Saved queries + sparklines
  let savedQs = $state<{ name: string; q: string }[]>(loadSaved());
  let sparks = $state<Record<string, number[]>>({});
  function loadSaved(): { name: string; q: string }[] {
    try { return JSON.parse(ls?.getItem("anvil-prom-queries") || "[]"); } catch { return []; }
  }
  function persistSaved() { ls?.setItem("anvil-prom-queries", JSON.stringify(savedQs)); }
  async function saveQuery() {
    if (!promQuery.trim()) return;
    const name = await askText({ title: "Save query", value: promQuery.slice(0, 40) });
    if (!name) return;
    savedQs = [...savedQs.filter((x) => x.q !== promQuery), { name, q: promQuery }];
    persistSaved(); loadSparks();
  }
  function removeQuery(q: string) { savedQs = savedQs.filter((x) => x.q !== q); persistSaved(); delete sparks[q]; }
  async function loadSparks() {
    if (!promBase || !savedQs.length) return;
    for (const { q } of savedQs) {
      try {
        const j = JSON.parse(await invoke<string>("prom_query_range", { base: promBase, query: q, minutes: 60 }));
        sparks[q] = (j.data?.result?.[0]?.values ?? []).map((v: any) => parseFloat(v[1])).filter((n: number) => !Number.isNaN(n));
      } catch { sparks[q] = []; }
    }
  }
  function sparkLast(vals: number[]): string {
    if (!vals?.length) return "—";
    const n = vals[vals.length - 1];
    return Math.abs(n) >= 1000 || (n !== 0 && Math.abs(n) < 0.01) ? n.toExponential(2) : n.toFixed(2);
  }
  function sparkPath(vals: number[], w = 120, h = 22): string {
    if (!vals || vals.length < 2) return "";
    const min = Math.min(...vals), max = Math.max(...vals), span = max - min || 1;
    return vals.map((v, i) => `${i === 0 ? "M" : "L"}${((i / (vals.length - 1)) * w).toFixed(1)} ${(h - ((v - min) / span) * h).toFixed(1)}`).join(" ");
  }

  // Firing alerts
  let alerts = $state<{ name: string; sev: string; labels: string }[]>([]);
  let alertErr = $state("");
  async function loadAlerts() {
    if (!promBase) return;
    alertErr = "";
    try {
      const j = JSON.parse(await invoke<string>("prom_query", { base: promBase, query: 'ALERTS{alertstate="firing"}' }));
      if (j.status !== "success") { alertErr = j.error || "query error"; alerts = []; return; }
      alerts = (j.data?.result ?? []).map((r: any) => {
        const m = r.metric ?? {};
        return {
          name: m.alertname || "alert",
          sev: m.severity || "",
          labels: Object.entries(m).filter(([k]) => !["alertname", "alertstate", "severity", "__name__"].includes(k)).map(([k, v]) => `${k}=${v}`).join("  "),
        };
      });
    } catch (e) { alertErr = String(e); alerts = []; }
  }
  function refreshMetrics() { loadAlerts(); loadSparks(); }

  // ── Loki ──
  let lokiBase = $state(ls?.getItem("anvil-loki-base") ?? "");
  let lokiQuery = $state("");
  let lokiLines = $state<{ ts: string; line: string }[]>([]);
  let lokiErr = $state("");
  let lokiTail = $state(false);
  let lokiTimer: ReturnType<typeof setInterval> | null = null;

  async function runLoki() {
    if (!lokiBase || !lokiQuery) return;
    ls?.setItem("anvil-loki-base", lokiBase);
    lokiErr = "";
    try {
      const j = JSON.parse(await invoke<string>("loki_query", { base: lokiBase, query: lokiQuery }));
      if (j.status !== "success") { lokiErr = j.error || "query error"; return; }
      const out: { ts: string; line: string }[] = [];
      for (const s of j.data?.result ?? []) {
        for (const [ns, line] of s.values ?? []) out.push({ ts: new Date(Number(ns) / 1e6).toLocaleTimeString(), line });
      }
      out.sort((a, b) => (a.ts < b.ts ? 1 : -1));
      lokiLines = out.slice(0, 300);
      if (!lokiLines.length) lokiErr = "no log lines";
    } catch (e) { lokiErr = String(e); }
  }
  function toggleLokiTail() {
    lokiTail = !lokiTail;
    if (lokiTimer) { clearInterval(lokiTimer); lokiTimer = null; }
    if (lokiTail) { runLoki(); lokiTimer = setInterval(runLoki, 3000); }
  }
  onDestroy(() => { if (lokiTimer) clearInterval(lokiTimer); });
</script>

<div class="obs">
  <div class="topbar">
    <div class="seg">
      <button class="seg-btn" class:on={view === "metrics"} onclick={() => (view = "metrics")}>Metrics</button>
      <button class="seg-btn" class:on={view === "logs"} onclick={() => (view = "logs")}>Logs</button>
    </div>
    <span class="spacer"></span>
    {#if view === "metrics"}
      <button class="iconbtn" onclick={refreshMetrics} title="Refresh alerts + sparklines"><Icon name="refresh" size={13} /></button>
    {/if}
  </div>

  {#if view === "metrics"}
    <div class="bar">
      <input class="in" placeholder="Prometheus base URL (http://host:9090)" bind:value={promBase} />
      <input class="in mono" placeholder="PromQL  e.g. up" bind:value={promQuery}
        onkeydown={(e) => e.key === "Enter" && runProm()} />
      <button class="btn" onclick={runProm} disabled={promBusy}>{promBusy ? "…" : "Run"}</button>
      <button class="btn" onclick={saveQuery} title="Save query">Save</button>
    </div>

    <div class="scroll">
      {#if alerts.length}
        <div class="sec-h">Firing alerts <span class="cnt">{alerts.length}</span></div>
        {#each alerts as a (a.name + a.labels)}
          <div class="alert">
            <span class="adot" class:crit={a.sev === "critical"}></span>
            <span class="aname">{a.name}</span>
            {#if a.sev}<span class="asev">{a.sev}</span>{/if}
            <span class="albl">{a.labels}</span>
          </div>
        {/each}
      {/if}
      {#if alertErr}<div class="err">{alertErr.slice(0, 160)}</div>{/if}

      {#if savedQs.length}
        <div class="sec-h">Saved</div>
        {#each savedQs as s (s.q)}
          <div class="spark" role="button" tabindex="0" title={s.q}
            onclick={() => { promQuery = s.q; runProm(); }} onkeydown={(e) => e.key === "Enter" && (promQuery = s.q)}>
            <span class="sname">{s.name}</span>
            <svg class="sparksvg" viewBox="0 0 120 22" preserveAspectRatio="none"><path d={sparkPath(sparks[s.q] ?? [])} fill="none" stroke="var(--accent)" stroke-width="1.2" /></svg>
            <span class="sval">{sparkLast(sparks[s.q] ?? [])}</span>
            <button class="xbtn" onclick={(e) => { e.stopPropagation(); removeQuery(s.q); }} title="Remove"><Icon name="close" size={10} /></button>
          </div>
        {/each}
      {/if}

      {#if promErr}<div class="err">{promErr.slice(0, 200)}</div>{/if}
      {#if promRows.length}
        <div class="sec-h">Result <span class="cnt">{promRows.length}</span></div>
        {#each promRows as r (r.metric)}
          <div class="mrow"><span class="mmetric mono">{r.metric}</span><span class="mval mono">{r.value}</span></div>
        {/each}
      {/if}
    </div>
  {:else}
    <div class="bar">
      <input class="in" placeholder="Loki base URL (http://host:3100)" bind:value={lokiBase} />
      <input class="in mono" placeholder={'LogQL  e.g. {app="api"} |= "error"'} bind:value={lokiQuery}
        onkeydown={(e) => e.key === "Enter" && runLoki()} />
      <button class="btn" onclick={runLoki}>Run</button>
      <button class="btn" class:on={lokiTail} onclick={toggleLokiTail} title="Live tail (3s)">{lokiTail ? "Tailing" : "Tail"}</button>
    </div>
    {#if lokiErr}<div class="err">{lokiErr.slice(0, 200)}</div>{/if}
    <div class="logout">
      {#each lokiLines as l (l.ts + l.line)}
        <div class="lrow"><span class="lts">{l.ts}</span><span class="lline">{l.line}</span></div>
      {/each}
      {#if !lokiLines.length && !lokiErr}<div class="empty">Run a LogQL query to stream logs.</div>{/if}
    </div>
  {/if}
</div>

<style>
  .obs { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .topbar { display: flex; align-items: center; gap: 10px; height: 32px; flex: 0 0 auto; padding: 0 12px; border-bottom: 1px solid var(--border); }
  .seg { display: inline-flex; border: 1px solid var(--border); border-radius: 5px; overflow: hidden; }
  .seg-btn { padding: 2px 11px; height: 21px; border: 0; background: transparent; color: var(--text3); font-size: 11px; cursor: default; border-right: 1px solid var(--border); }
  .seg-btn:last-child { border-right: 0; }
  .seg-btn:hover { color: var(--text); }
  .seg-btn.on { background: var(--sel); color: var(--text); }
  .spacer { flex: 1; }
  .iconbtn { display: inline-flex; align-items: center; justify-content: center; width: 22px; height: 20px; border: 0; border-radius: 5px; background: transparent; color: var(--text3); cursor: default; }
  .iconbtn:hover { background: var(--sel); color: var(--text); }

  .bar { display: flex; align-items: center; gap: 6px; padding: 7px 12px; border-bottom: 1px solid var(--border); flex: 0 0 auto; }
  .in {
    height: 24px; border: 1px solid var(--border); background: var(--panel2); color: var(--text);
    border-radius: 5px; padding: 0 8px; font-size: 11.5px; font-family: var(--font-ui);
  }
  .in:first-child { flex: 0 0 230px; }
  .in.mono { flex: 1; font-family: var(--font-mono); font-size: 11px; }
  .in:focus { outline: none; border-color: var(--text3); }
  .btn {
    height: 24px; padding: 0 11px; border: 1px solid var(--border); background: var(--panel2);
    color: var(--text2); border-radius: 5px; font-size: 11.5px; cursor: default; flex: 0 0 auto;
  }
  .btn:hover:not(:disabled) { color: var(--text); border-color: var(--text3); }
  .btn:disabled { opacity: 0.5; }
  .btn.on { color: var(--accent); border-color: var(--accent); }

  .scroll { flex: 1; min-height: 0; overflow-y: auto; }
  .sec-h {
    display: flex; align-items: center; gap: 7px; padding: 7px 12px 4px; font-size: 10px; font-weight: 500;
    color: var(--text3); text-transform: uppercase; letter-spacing: 0.05em;
  }
  .cnt { font-family: var(--font-mono); font-size: 9.5px; opacity: 0.7; letter-spacing: 0; }
  .err { padding: 8px 12px; font-size: 11.5px; color: var(--red); font-family: var(--font-mono); }
  .empty { padding: 18px 14px; color: var(--text3); font-size: 12px; }

  .alert { display: flex; align-items: center; gap: 8px; padding: 0 12px; height: 24px; border-bottom: 1px solid var(--hairline); font-size: 11.5px; }
  .adot { width: 7px; height: 7px; border-radius: 50%; background: var(--yellow); flex: 0 0 auto; }
  .adot.crit { background: var(--red); }
  .aname { color: var(--text); font-weight: 500; }
  .asev { font-family: var(--font-mono); font-size: 9.5px; color: var(--text3); border: 1px solid var(--border); border-radius: 3px; padding: 0 4px; }
  .albl { color: var(--text3); font-family: var(--font-mono); font-size: 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .spark { display: flex; align-items: center; gap: 10px; padding: 0 12px; height: 30px; border-bottom: 1px solid var(--hairline); cursor: default; }
  .spark:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }
  .sname { flex: 0 0 130px; font-size: 11.5px; color: var(--text2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sparksvg { flex: 1; height: 22px; min-width: 0; }
  .sval { flex: 0 0 auto; font-family: var(--font-mono); font-size: 11px; color: var(--text); }
  .xbtn { border: 0; background: transparent; color: var(--text3); cursor: default; display: inline-flex; padding: 2px; }
  .xbtn:hover { color: var(--red); }

  .mrow { display: flex; align-items: baseline; gap: 12px; padding: 3px 12px; border-bottom: 1px solid var(--hairline); font-size: 11px; }
  .mmetric { flex: 1; min-width: 0; color: var(--text2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mval { flex: 0 0 auto; color: var(--text); }
  .mono { font-family: var(--font-mono); }

  .logout { flex: 1; min-height: 0; overflow: auto; background: var(--bg); padding: 4px 0; }
  .lrow { display: flex; gap: 10px; padding: 0 12px; font-family: var(--font-mono); font-size: 11px; line-height: 1.55; white-space: pre; min-width: max-content; }
  .lrow:hover { background: color-mix(in srgb, var(--text) 4%, transparent); }
  .lts { color: var(--text3); opacity: 0.55; flex: 0 0 auto; user-select: none; }
  .lline { color: var(--text2); }
</style>
