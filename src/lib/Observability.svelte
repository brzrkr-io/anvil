<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import { askText } from "$lib/dialog";
  import { toast } from "$lib/toast";

  const ls = typeof localStorage !== "undefined" ? localStorage : null;
  function lsGet(k: string, def = ""): string { return ls?.getItem(k) ?? def; }
  function lsSet(k: string, v: string) { ls?.setItem(k, v); }

  let view = $state<"metrics" | "signoz" | "dashboards">("metrics");

  // API keys live in the macOS Keychain, never localStorage.
  async function loadKey(name: string): Promise<string> {
    try { return await invoke<string>("secret_get", { key: name }); } catch { return ""; }
  }
  async function saveKey(name: string, value: string) {
    try { await invoke("secret_set", { key: name, value }); } catch (e) { toast("Keychain save failed: " + String(e).slice(0, 80), "error"); }
  }
  function openUrl(url: string) {
    invoke("open_url_window", { url }).catch((e) => toast(String(e).slice(0, 100), "error"));
  }

  // ── Prometheus (metrics) ──
  let promBase = $state(lsGet("anvil-prom-base", "https://prometheus.firemon.net"));
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
    lsSet("anvil-prom-base", promBase);
    promErr = ""; promRows = []; promBusy = true;
    try {
      const j = JSON.parse(await invoke<string>("prom_query", { base: promBase, query: promQuery }));
      if (j.status !== "success") { promErr = j.error || "query error"; return; }
      promRows = (j.data?.result ?? []).map((r: any) => ({ metric: metricLabel(r.metric ?? {}), value: r.value?.[1] ?? "" }));
      if (!promRows.length) promErr = "no matching series";
    } catch (e) { promErr = String(e); } finally { promBusy = false; }
  }

  let savedQs = $state<{ name: string; q: string }[]>(loadSaved());
  let sparks = $state<Record<string, number[]>>({});
  function loadSaved(): { name: string; q: string }[] {
    try { return JSON.parse(lsGet("anvil-prom-queries", "[]")); } catch { return []; }
  }
  function persistSaved() { lsSet("anvil-prom-queries", JSON.stringify(savedQs)); }
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
  let alerts = $state<{ name: string; sev: string; labels: string }[]>([]);
  async function loadAlerts() {
    if (!promBase) return;
    try {
      const j = JSON.parse(await invoke<string>("prom_query", { base: promBase, query: 'ALERTS{alertstate="firing"}' }));
      if (j.status !== "success") { alerts = []; return; }
      alerts = (j.data?.result ?? []).map((r: any) => {
        const m = r.metric ?? {};
        return {
          name: m.alertname || "alert", sev: m.severity || "",
          labels: Object.entries(m).filter(([k]) => !["alertname", "alertstate", "severity", "__name__"].includes(k)).map(([k, v]) => `${k}=${v}`).join("  "),
        };
      });
    } catch { alerts = []; }
  }
  function refreshMetrics() { loadAlerts(); loadSparks(); }

  // ── SigNoz (logs / traces / services) ──
  let sigBase = $state(lsGet("anvil-signoz-base", "https://signoz.firemon.net"));
  let sigKey = $state("");
  let sigServices = $state<{ serviceName: string; p99: number; errorRate: number; callRate: number }[]>([]);
  let sigErr = $state("");
  let sigBusy = $state(false);

  async function loadServices() {
    if (!sigBase) return;
    lsSet("anvil-signoz-base", sigBase);
    sigErr = ""; sigBusy = true;
    try {
      const raw = await invoke<string>("signoz_services", { base: sigBase, apiKey: sigKey });
      const arr = JSON.parse(raw);
      sigServices = (Array.isArray(arr) ? arr : []).map((s: any) => ({
        serviceName: s.serviceName ?? s.name ?? "",
        p99: Math.round((s.p99 ?? 0) / 1e6),
        errorRate: +(s.errorRate ?? 0).toFixed(2),
        callRate: +(s.callRate ?? 0).toFixed(2),
      })).filter((s: any) => s.serviceName);
      if (!sigServices.length) sigErr = "no services in the last 5m";
    } catch (e) { sigErr = String(e); } finally { sigBusy = false; }
  }

  // ── Grafana (dashboards) ──
  let grafBase = $state(lsGet("anvil-grafana-base", ""));
  let grafKey = $state("");
  let dashboards = $state<{ title: string; url: string; folderTitle?: string; tags?: string[] }[]>([]);
  let grafErr = $state("");
  let grafBusy = $state(false);

  async function loadDashboards() {
    if (!grafBase) { grafErr = "Set the Grafana base URL"; return; }
    lsSet("anvil-grafana-base", grafBase);
    grafErr = ""; grafBusy = true;
    try {
      const raw = await invoke<string>("grafana_dashboards", { base: grafBase, token: grafKey });
      dashboards = (JSON.parse(raw) as any[]).map((d) => ({ title: d.title, url: d.url, folderTitle: d.folderTitle, tags: d.tags }));
      if (!dashboards.length) grafErr = "no dashboards found";
    } catch (e) { grafErr = String(e); } finally { grafBusy = false; }
  }
  function openDashboard(url: string) { openUrl(grafBase.replace(/\/$/, "") + url); }

  onMount(async () => {
    sigKey = await loadKey("signoz-key");
    grafKey = await loadKey("grafana-token");
    loadAlerts();
    if (savedQs.length) loadSparks();
  });
</script>

<div class="obs">
  <div class="topbar">
    <div class="seg">
      <button class="seg-btn" class:on={view === "metrics"} onclick={() => (view = "metrics")}>Metrics</button>
      <button class="seg-btn" class:on={view === "signoz"} onclick={() => { view = "signoz"; if (!sigServices.length) loadServices(); }}>SigNoz</button>
      <button class="seg-btn" class:on={view === "dashboards"} onclick={() => { view = "dashboards"; if (!dashboards.length) loadDashboards(); }}>Dashboards</button>
    </div>
    <span class="spacer"></span>
    {#if view === "metrics"}<button class="iconbtn" onclick={refreshMetrics} title="Refresh alerts + sparklines"><Icon name="refresh" size={13} /></button>{/if}
    {#if view === "signoz"}<button class="iconbtn" onclick={loadServices} title="Refresh services"><Icon name="refresh" size={13} /></button>{/if}
    {#if view === "dashboards"}<button class="iconbtn" onclick={loadDashboards} title="Refresh dashboards"><Icon name="refresh" size={13} /></button>{/if}
  </div>

  {#if view === "metrics"}
    <div class="bar">
      <input class="in url" placeholder="Prometheus base URL" bind:value={promBase} />
      <input class="in mono" placeholder="PromQL  e.g. up" bind:value={promQuery} onkeydown={(e) => e.key === "Enter" && runProm()} />
      <button class="btn" onclick={runProm} disabled={promBusy}>{promBusy ? "…" : "Run"}</button>
      <button class="btn" onclick={saveQuery}>Save</button>
    </div>
    <div class="scroll">
      {#if alerts.length}
        <div class="sec-h">Firing alerts <span class="cnt">{alerts.length}</span></div>
        {#each alerts as a (a.name + a.labels)}
          <div class="alert"><span class="adot" class:crit={a.sev === "critical"}></span><span class="aname">{a.name}</span>{#if a.sev}<span class="asev">{a.sev}</span>{/if}<span class="albl">{a.labels}</span></div>
        {/each}
      {/if}
      {#if savedQs.length}
        <div class="sec-h">Saved</div>
        {#each savedQs as s (s.q)}
          <div class="spark" role="button" tabindex="0" title={s.q} onclick={() => { promQuery = s.q; runProm(); }} onkeydown={(e) => e.key === "Enter" && (promQuery = s.q)}>
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
  {:else if view === "signoz"}
    <div class="bar">
      <input class="in url" placeholder="SigNoz base URL" bind:value={sigBase} />
      <input class="in mono" type="password" placeholder="SIGNOZ-API-KEY (Keychain)" bind:value={sigKey}
        onchange={() => saveKey("signoz-key", sigKey)} />
      <button class="btn" onclick={loadServices} disabled={sigBusy}>{sigBusy ? "…" : "Services"}</button>
      <button class="btn ext" onclick={() => openUrl(sigBase)}>Open SigNoz ↗</button>
    </div>
    <div class="scroll">
      {#if sigErr}<div class="err">{sigErr.slice(0, 240)}</div>{/if}
      {#if sigServices.length}
        <div class="svc-head"><span>Service</span><span>p99 (ms)</span><span>err/s</span><span>req/s</span></div>
        {#each sigServices as s (s.serviceName)}
          <div class="svc-row">
            <span class="svc-name mono">{s.serviceName}</span>
            <span class="svc-n">{s.p99}</span>
            <span class="svc-n" style="color:{s.errorRate > 0 ? 'var(--red)' : 'var(--text3)'}">{s.errorRate}</span>
            <span class="svc-n">{s.callRate}</span>
          </div>
        {/each}
      {:else if !sigErr && !sigBusy}
        <div class="empty">Services overview (last 5m). Use <b>Open SigNoz ↗</b> for logs &amp; traces.</div>
      {/if}
    </div>
  {:else}
    <div class="bar">
      <input class="in url" placeholder="Grafana base URL (https://grafana.…)" bind:value={grafBase} />
      <input class="in mono" type="password" placeholder="Grafana token (Keychain)" bind:value={grafKey}
        onchange={() => saveKey("grafana-token", grafKey)} />
      <button class="btn" onclick={loadDashboards} disabled={grafBusy}>{grafBusy ? "…" : "Load"}</button>
    </div>
    <div class="scroll">
      {#if grafErr}<div class="err">{grafErr.slice(0, 240)}</div>{/if}
      {#each dashboards as d (d.url)}
        <div class="dash" role="button" tabindex="0" onclick={() => openDashboard(d.url)} onkeydown={(e) => e.key === "Enter" && openDashboard(d.url)}>
          <span class="dash-title">{d.title}</span>
          {#if d.folderTitle}<span class="dash-folder">{d.folderTitle}</span>{/if}
          {#each (d.tags ?? []).slice(0, 3) as t}<span class="dash-tag">{t}</span>{/each}
          <span class="dash-open"><Icon name="zoom" size={11} /></span>
        </div>
      {/each}
      {#if !dashboards.length && !grafErr && !grafBusy}
        <div class="empty">Set the Grafana URL + token and click <b>Load</b>. Dashboards open in their own window.</div>
      {/if}
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
  .in { height: 24px; border: 1px solid var(--border); background: var(--panel2); color: var(--text); border-radius: 5px; padding: 0 8px; font-size: 11.5px; font-family: var(--font-ui); }
  .in.url { flex: 0 0 240px; }
  .in.mono { flex: 1; font-family: var(--font-mono); font-size: 11px; }
  .in:focus { outline: none; border-color: var(--text3); }
  .btn { height: 24px; padding: 0 11px; border: 1px solid var(--border); background: var(--panel2); color: var(--text2); border-radius: 5px; font-size: 11.5px; cursor: default; flex: 0 0 auto; }
  .btn:hover:not(:disabled) { color: var(--text); border-color: var(--text3); }
  .btn:disabled { opacity: 0.5; }
  .btn.ext { font-family: var(--font-mono); font-size: 11px; }

  .scroll { flex: 1; min-height: 0; overflow-y: auto; }
  .sec-h { display: flex; align-items: center; gap: 7px; padding: 7px 12px 4px; font-size: 10px; font-weight: 500; color: var(--text3); text-transform: uppercase; letter-spacing: 0.05em; }
  .cnt { font-family: var(--font-mono); font-size: 9.5px; opacity: 0.7; letter-spacing: 0; }
  .err { padding: 8px 12px; font-size: 11.5px; color: var(--red); font-family: var(--font-mono); white-space: pre-wrap; }
  .empty { padding: 18px 14px; color: var(--text3); font-size: 12px; line-height: 1.5; }

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

  .svc-head, .svc-row { display: grid; grid-template-columns: minmax(0,1fr) 70px 60px 60px; column-gap: 10px; align-items: center; padding: 0 12px; height: 24px; border-bottom: 1px solid var(--hairline); font-size: 11.5px; }
  .svc-head { color: var(--text3); font-size: 10px; text-transform: uppercase; letter-spacing: 0.04em; position: sticky; top: 0; background: var(--panel); }
  .svc-head span:not(:first-child), .svc-n { text-align: right; font-family: var(--font-mono); }
  .svc-row:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }
  .svc-name { color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .dash { display: flex; align-items: center; gap: 8px; padding: 0 12px; height: 28px; border-bottom: 1px solid var(--hairline); cursor: default; font-size: 12px; }
  .dash:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }
  .dash-title { color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dash-folder { font-size: 10px; color: var(--text3); flex: 0 0 auto; }
  .dash-tag { font-family: var(--font-mono); font-size: 9px; color: var(--text3); border: 1px solid var(--border); border-radius: 3px; padding: 0 4px; flex: 0 0 auto; }
  .dash-open { margin-left: auto; color: var(--text3); flex: 0 0 auto; }
  .dash:hover .dash-open { color: var(--text); }
</style>
