<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import { toast } from "$lib/toast";
  import { parseContainers, runningCount, type Container } from "$lib/docker-ps";

  let { onRunCommand }: { onRunCommand?: (cmd: string) => void } = $props();

  let items = $state<Container[]>([]);
  let all = $state(false);
  let err = $state("");
  let loading = $state(false);
  let busyRow = $state("");
  const running = $derived(runningCount(items));
  const daemonErr = $derived(/not found|Cannot connect|daemon/i.test(err));

  async function load() {
    loading = true;
    err = "";
    try {
      items = parseContainers(await invoke<string>("docker_ps", { all }));
    } catch (e) {
      err = String(e);
      items = [];
    } finally {
      loading = false;
    }
  }

  async function act(c: Container, action: "start" | "stop" | "restart" | "rm") {
    if (action === "rm" && !confirm(`Remove container ${c.name}? (force)`)) return;
    busyRow = c.id;
    try {
      await invoke("docker_action", { id: c.id, action });
      toast(`${action} ${c.name}`, "success");
      await load();
    } catch (e) {
      toast(String(e).slice(0, 140), "error");
    } finally {
      busyRow = "";
    }
  }

  const logs = (c: Container) => onRunCommand?.(`docker logs -f --tail=200 ${c.id}`);
  const shell = (c: Container) => onRunCommand?.(`docker exec -it ${c.id} sh -c 'command -v bash >/dev/null && exec bash || exec sh'`);

  onMount(load);
</script>

<div class="dk">
  <div class="bar">
    <span class="lbl">Containers · {running} running</span>
    <span class="grow"></span>
    <button class="t" class:on={all} onclick={() => { all = !all; load(); }} title="Include stopped containers">all</button>
    <button class="t" onclick={load} title="Refresh (docker ps)" disabled={loading}><Icon name="refresh" size={13} /></button>
  </div>

  {#if err}
    <pre class="out">{daemonErr ? "Docker not running or not installed." : err.slice(0, 200)}</pre>
  {:else if items.length}
    <div class="list">
      {#each items as c (c.id)}
        <div class="row" class:busy={busyRow === c.id}>
          <span class="dot s-{c.state}" title={c.status}></span>
          <span class="nm" title={c.image}>{c.name}</span>
          <span class="img">{c.image}</span>
          <span class="ports" title={c.ports}>{c.ports}</span>
          <span class="grow"></span>
          <button class="b" title="Logs in terminal" onclick={() => logs(c)}><Icon name="terminal" size={11} /></button>
          {#if c.state === "running"}
            <button class="b" title="Shell into container" onclick={() => shell(c)}>↳</button>
            <button class="b" title="Stop" disabled={!!busyRow} onclick={() => act(c, "stop")}><Icon name="minus" size={11} /></button>
            <button class="b" title="Restart" disabled={!!busyRow} onclick={() => act(c, "restart")}><Icon name="refresh" size={11} /></button>
          {:else}
            <button class="b" title="Start" disabled={!!busyRow} onclick={() => act(c, "start")}><Icon name="play" size={11} /></button>
            <button class="b danger" title="Remove" disabled={!!busyRow} onclick={() => act(c, "rm")}>✕</button>
          {/if}
        </div>
      {/each}
    </div>
  {:else}
    <pre class="out">{loading ? "Loading…" : all ? "No containers." : "No running containers — toggle 'all'."}</pre>
  {/if}
</div>

<style>
  .dk { display: flex; flex-direction: column; flex: 1; min-height: 0; }
  .bar { display: flex; align-items: center; gap: 8px; height: 32px; flex: 0 0 auto; padding: 0 12px; border-bottom: 1px solid var(--border); }
  .lbl { font-size: 11.5px; color: var(--text3); text-transform: uppercase; letter-spacing: 0.04em; }
  .grow { flex: 1; }
  .t { background: transparent; border: 1px solid var(--border); color: var(--text3); border-radius: 5px; padding: 2px 8px; font-size: 11px; cursor: default; display: inline-flex; align-items: center; }
  .t.on { color: var(--text); background: var(--sel); }
  .t:hover:not(:disabled) { color: var(--text); }
  .list { flex: 1; min-height: 0; overflow-y: auto; }
  .row { display: flex; align-items: center; gap: 8px; padding: 3px 12px; height: 26px; font-size: 12px; }
  .row:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }
  .row.busy { opacity: 0.5; }
  .dot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; background: var(--text3); }
  .dot.s-running { background: var(--green); }
  .dot.s-exited { background: var(--text3); }
  .dot.s-paused { background: var(--yellow); }
  .dot.s-other { background: var(--text3); }
  .nm { color: var(--text); font-family: var(--font-mono); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 200px; }
  .img { color: var(--text3); font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 180px; }
  .ports { color: var(--text2); font-size: 10.5px; font-family: var(--font-mono); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 160px; }
  .b { background: transparent; border: 1px solid var(--border); color: var(--text2); border-radius: 5px; min-width: 22px; height: 20px; padding: 0 5px; font-size: 11px; display: inline-flex; align-items: center; justify-content: center; cursor: default; }
  .b:hover:not(:disabled) { color: var(--text); border-color: var(--text3); }
  .b:disabled { opacity: 0.4; }
  .b.danger:hover:not(:disabled) { color: var(--red); border-color: var(--red); }
  .out { margin: 10px 12px; color: var(--text3); font-size: 12px; font-family: var(--font-mono); white-space: pre-wrap; }
</style>
