<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import { toast } from "$lib/toast";

  let { onRunCommand, onPresence }: { onRunCommand?: (cmd: string) => void; onPresence?: (present: boolean) => void } = $props();

  type Tab = "kustomizations" | "helmreleases" | "sources";
  interface FluxItem {
    name: string;
    ns: string;
    apiKind: string; // Kustomization | HelmRelease | GitRepository | ...
    ready: "ok" | "fail" | "unknown";
    suspended: boolean;
    revision: string;
    message: string;
  }

  let tab = $state<Tab>("kustomizations");
  let items = $state<FluxItem[]>([]);
  let loading = $state(false);
  let err = $state("");
  let present = $state(true); // false → cluster has no Flux CRDs; hide the panel
  let busyRow = $state("");

  const TABS: { id: Tab; label: string }[] = [
    { id: "kustomizations", label: "Kustomizations" },
    { id: "helmreleases", label: "HelmReleases" },
    { id: "sources", label: "Sources" },
  ];

  function shortRev(r: string): string {
    // git "main@sha1:abcd1234..." or "sha256:..." → keep branch + 7 chars.
    const m = r.match(/^(.*?[@:])?([0-9a-f]{7,})/i);
    if (m) return `${m[1] ?? ""}${m[2].slice(0, 7)}`;
    return r.length > 24 ? r.slice(0, 24) + "…" : r;
  }

  // Map the k8s object kind → the `flux` CLI kind argument the backend allow-lists.
  function fluxKind(apiKind: string): string {
    switch (apiKind) {
      case "Kustomization": return "kustomization";
      case "HelmRelease": return "helmrelease";
      case "GitRepository": return "source git";
      case "OCIRepository": return "source oci";
      case "HelmRepository": return "source helm";
      case "HelmChart": return "source chart";
      case "Bucket": return "source bucket";
      default: return "";
    }
  }

  function parse(raw: string): FluxItem[] {
    let j: any;
    try { j = JSON.parse(raw); } catch { return []; }
    const list = Array.isArray(j?.items) ? j.items : [];
    return list.map((it: any): FluxItem => {
      const conds = it?.status?.conditions ?? [];
      const ready = conds.find((c: any) => c.type === "Ready");
      const st = it?.status ?? {};
      return {
        name: it?.metadata?.name ?? "?",
        ns: it?.metadata?.namespace ?? "",
        apiKind: it?.kind ?? "",
        ready: !ready ? "unknown" : ready.status === "True" ? "ok" : "fail",
        suspended: it?.spec?.suspend === true,
        revision: st.lastAppliedRevision || st.lastAttemptedRevision || st.artifact?.revision || "",
        message: ready?.message ?? "",
      };
    });
  }

  async function load() {
    loading = true;
    err = "";
    try {
      const raw = await invoke<string>("flux_get", { kind: tab });
      if (/the server doesn't have a resource type|no matches for kind|NotFound|could not find the requested resource/i.test(raw)) {
        present = false;
        items = [];
        return;
      }
      present = true;
      items = parse(raw).sort((a, b) => (a.ns + a.name).localeCompare(b.ns + b.name));
    } catch (e) {
      err = String(e);
    } finally {
      loading = false;
      onPresence?.(present);
    }
  }

  async function act(it: FluxItem, cmd: "flux_reconcile" | "flux_suspend" | "flux_resume", withSource = false) {
    const kind = fluxKind(it.apiKind);
    if (!kind) return;
    if (cmd === "flux_suspend" && !confirm(`Suspend reconciliation of ${it.apiKind} ${it.ns}/${it.name}?`)) return;
    busyRow = `${it.ns}/${it.name}`;
    try {
      const args: Record<string, unknown> = { kind, name: it.name, namespace: it.ns };
      if (cmd === "flux_reconcile") args.withSource = withSource;
      const out = await invoke<string>(cmd, args);
      const verb = cmd.replace("flux_", "");
      toast(`${verb} ${it.name}: ${out.trim().split("\n").pop() || "done"}`.slice(0, 120), /error|fail/i.test(out) ? "error" : "success");
      await load();
    } catch (e) {
      toast(String(e).slice(0, 160), "error");
    } finally {
      busyRow = "";
    }
  }

  function logs(it: FluxItem) {
    onRunCommand?.(`flux logs --kind=${it.apiKind} --name=${it.name} -n ${it.ns} -f`);
  }

  onMount(load);
</script>

{#if present}
  <div class="flux">
    <div class="fx-tabs">
      {#each TABS as t (t.id)}
        <button class:on={tab === t.id} onclick={() => { tab = t.id; load(); }}>{t.label}</button>
      {/each}
      <span class="spacer"></span>
      {#if loading}<span class="spin">…</span>{/if}
      <button class="fx-refresh" title="Refresh" onclick={load}><Icon name="refresh" size={12} /></button>
    </div>

    {#if err}<div class="fx-err">{err.slice(0, 200)}</div>{/if}

    <div class="fx-body">
        {#if loading && !items.length}
          <div class="fx-empty">Loading…</div>
        {:else if !items.length}
          <div class="fx-empty">No {tab} found.</div>
        {:else}
          {#each items as it (it.ns + "/" + it.apiKind + "/" + it.name)}
            <div class="fx-row" class:busy={busyRow === it.ns + "/" + it.name}>
              <span class="fx-dot {it.suspended ? 'susp' : it.ready}" title={it.suspended ? "Suspended" : it.ready}></span>
              <span class="fx-name" title={it.message}>{it.name}</span>
              <span class="fx-ns">{it.ns}</span>
              {#if tab === "sources"}<span class="fx-k">{it.apiKind}</span>{/if}
              <span class="fx-rev mono" title={it.revision}>{shortRev(it.revision)}</span>
              <span class="spacer"></span>
              <button class="fx-act" title="Reconcile (sync now)" disabled={!!busyRow} onclick={() => act(it, "flux_reconcile")}><Icon name="refresh" size={12} /></button>
              {#if tab !== "sources"}
                <button class="fx-act" title="Reconcile with source" disabled={!!busyRow} onclick={() => act(it, "flux_reconcile", true)}>↻+</button>
              {/if}
              {#if it.suspended}
                <button class="fx-act" title="Resume" disabled={!!busyRow} onclick={() => act(it, "flux_resume")}><Icon name="play" size={12} /></button>
              {:else}
                <button class="fx-act" title="Suspend" disabled={!!busyRow} onclick={() => act(it, "flux_suspend")}><Icon name="minus" size={12} /></button>
              {/if}
              <button class="fx-act" title="Stream logs in terminal" onclick={() => logs(it)}>↗</button>
            </div>
          {/each}
        {/if}
      </div>
  </div>
{/if}

<style>
  .flux { display: flex; flex-direction: column; flex: 1; min-height: 0; }
  .spacer { flex: 1; }
  .spin { color: var(--text3); }
  .fx-tabs { display: flex; align-items: center; gap: 2px; padding: 7px var(--pad-x, 10px); border-bottom: 1px solid var(--border); flex: 0 0 auto; }
  .fx-tabs button { background: transparent; border: 1px solid transparent; color: var(--text3);
    font-family: var(--font-ui); font-size: 11.5px; padding: 3px 10px; border-radius: 5px; cursor: default; }
  .fx-tabs button:hover { color: var(--text2); }
  .fx-tabs button.on { color: var(--text); background: var(--panel2); border-color: var(--border); }
  .fx-refresh { color: var(--text3); display: inline-flex; align-items: center; padding: 3px; border: 0; background: transparent; border-radius: 4px; cursor: default; }
  .fx-refresh:hover { color: var(--text); background: color-mix(in srgb, var(--text) 8%, transparent); }
  .fx-err { margin: 6px var(--pad-x, 10px); color: var(--red); font-size: 11px; font-family: var(--font-mono); }
  .fx-body { flex: 1; min-height: 0; overflow-y: auto; }
  .fx-empty { padding: 10px var(--pad-x, 10px); color: var(--text3); font-size: 12px; }
  .fx-row { display: flex; align-items: center; gap: 8px; padding: 3px var(--pad-x, 10px); height: 24px; font-size: 12px; }
  .fx-row:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }
  .fx-row.busy { opacity: 0.5; }
  .fx-dot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; background: var(--text3); }
  .fx-dot.ok { background: var(--green); }
  .fx-dot.fail { background: var(--red); }
  .fx-dot.susp { background: var(--yellow); }
  .fx-name { color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 200px; }
  .fx-ns { color: var(--text3); font-size: 11px; }
  .fx-k { color: var(--accent); font-size: 10px; font-family: var(--font-mono); }
  .fx-rev { color: var(--text2); font-size: 10.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 200px; }
  .mono { font-family: var(--font-mono); }
  .fx-act { background: transparent; border: 1px solid var(--border); color: var(--text2); border-radius: 5px;
    min-width: 22px; height: 20px; padding: 0 5px; font-size: 11px; display: inline-flex; align-items: center; justify-content: center; cursor: default; }
  .fx-act:hover:not(:disabled) { color: var(--text); border-color: var(--text3); }
  .fx-act:disabled { opacity: 0.4; }
</style>
