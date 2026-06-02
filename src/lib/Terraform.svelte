<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "$lib/Icon.svelte";
  import { toast } from "$lib/toast";

  let { cwd, onRunCommand }: { cwd: string; onRunCommand?: (cmd: string) => void } = $props();

  type Bin = "terraform" | "terragrunt" | "tofu";
  interface Stack { path: string; terragrunt: boolean; }

  let stacks = $state<Stack[]>([]);
  let activeStack = $state<string | null>(null);
  let scanning = $state(false);

  let bin = $state<Bin>("terraform");
  let available = $state<Record<Bin, boolean>>({ terraform: true, terragrunt: false, tofu: false });
  let resources = $state<string[]>([]);
  let output = $state("");
  let outKind = $state<"plan" | "validate" | "init" | "">("");
  let running = $state("");
  let err = $state("");
  let outEl = $state<HTMLPreElement | null>(null);

  const BIN_LABEL: Record<Bin, string> = { terraform: "Terraform", terragrunt: "Terragrunt", tofu: "OpenTofu" };

  // Absolute dir the CLI runs in = repo root + selected stack.
  const activeDir = $derived(
    activeStack && activeStack !== "." ? `${cwd}/${activeStack}` : cwd,
  );

  async function discover() {
    scanning = true;
    err = "";
    try {
      const raw = await invoke<string>("tf_discover", { cwd });
      stacks = JSON.parse(raw) as Stack[];
      if (stacks.length && (!activeStack || !stacks.find((s) => s.path === activeStack))) {
        // Prefer a leaf stack over the root; pick the shallowest path.
        await selectStack(stacks[0].path);
      } else if (!stacks.length) {
        activeStack = null;
        resources = [];
      }
    } catch (e) {
      err = String(e);
    } finally {
      scanning = false;
    }
  }

  async function selectStack(path: string) {
    activeStack = path;
    output = "";
    outKind = "";
    resources = [];
    const st = stacks.find((s) => s.path === path);
    // Bias the binary toggle to what the stack actually uses.
    if (st?.terragrunt && available.terragrunt) bin = "terragrunt";
    await detect();
    await loadState();
  }

  async function detect() {
    try {
      const raw = await invoke<string>("tf_detect", { cwd: activeDir });
      const d = JSON.parse(raw) as { prefer: Bin; terraform: boolean; terragrunt: boolean; tofu: boolean };
      available = { terraform: d.terraform, terragrunt: d.terragrunt, tofu: d.tofu };
      if (available[d.prefer]) bin = d.prefer;
      else bin = (["terraform", "terragrunt", "tofu"] as Bin[]).find((b) => available[b]) ?? "terraform";
    } catch (e) {
      err = String(e);
    }
  }

  async function loadState() {
    if (!activeStack) return;
    err = "";
    try {
      const raw = await invoke<string>("tf_state_list", { cwd: activeDir, bin });
      if (/No state file|Backend initialization|not been initialized|reinitialization required/i.test(raw)) {
        resources = [];
        return;
      }
      resources = raw.split("\n").map((l) => l.trim()).filter(Boolean);
    } catch (e) {
      err = String(e);
      resources = [];
    }
  }

  async function run(cmd: "init" | "validate" | "plan", kind: typeof outKind) {
    if (running || !activeStack) return;
    running = cmd;
    output = "";
    outKind = kind;
    err = "";
    try {
      const map = { init: "tf_init", validate: "tf_validate", plan: "tf_plan" } as const;
      output = await invoke<string>(map[cmd], { cwd: activeDir, bin });
      if (cmd === "init" || cmd === "plan") await loadState();
      if (outEl) outEl.scrollTop = 0;
    } catch (e) {
      err = String(e);
    } finally {
      running = "";
    }
  }

  function sendTerminal(verb: "apply" | "destroy") {
    const cd = activeStack && activeStack !== "." ? `cd ${activeDir} && ` : "";
    onRunCommand?.(`${cd}${bin} ${verb}`);
    toast(`Sent "${bin} ${verb}" to terminal`, "info");
  }

  // Group state addresses by module path so big states stay scannable.
  interface Group { module: string; items: string[]; }
  const groups = $derived.by<Group[]>(() => {
    const m = new Map<string, string[]>();
    for (const addr of resources) {
      const mod = addr.match(/^((?:module\.[^.]+\.)+)/);
      const key = mod ? mod[1].replace(/\.$/, "") : "(root)";
      const res = mod ? addr.slice(mod[1].length) : addr;
      const arr = m.get(key) ?? [];
      arr.push(res);
      m.set(key, arr);
    }
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0])).map(([module, items]) => ({ module, items }));
  });

  let collapsed = $state<Record<string, boolean>>({});
  function toggle(k: string) { collapsed = { ...collapsed, [k]: !collapsed[k] }; }

  // Plan summary: "Plan: 3 to add, 1 to change, 2 to destroy."
  const summary = $derived.by(() => {
    const m = output.match(/Plan:\s+(\d+)\s+to add,\s+(\d+)\s+to change,\s+(\d+)\s+to destroy/);
    if (m) return { add: +m[1], change: +m[2], destroy: +m[3], none: false };
    if (outKind === "plan" && /No changes\.|infrastructure matches/i.test(output)) {
      return { add: 0, change: 0, destroy: 0, none: true };
    }
    return null;
  });

  function lineClass(l: string): string {
    const t = l.replace(/^\s+/, "");
    if (t.startsWith("-/+") || t.startsWith("+/-")) return "rec";
    if (t.startsWith("+")) return "add";
    if (t.startsWith("- ") || t === "-") return "del";
    if (t.startsWith("~")) return "chg";
    if (/^(Error:|╷|│\s*Error|✗)/.test(t)) return "err";
    if (/^(Success!|✓|Apply complete|No changes)/.test(t)) return "ok";
    return "";
  }

  function stackLabel(p: string): string {
    return p === "." ? "(repo root)" : p;
  }

  onMount(discover);

  // Re-scan when the repo dir changes.
  let lastCwd = "";
  $effect(() => {
    if (cwd && cwd !== lastCwd) {
      lastCwd = cwd;
      activeStack = null;
      discover();
    }
  });
</script>

<div class="tf">
  <div class="topbar">
    <div class="seg">
      {#each ["terraform", "terragrunt", "tofu"] as const as b}
        <button
          class="seg-btn"
          class:on={bin === b}
          disabled={!available[b]}
          title={available[b] ? BIN_LABEL[b] : `${BIN_LABEL[b]} not in PATH`}
          onclick={() => { bin = b; loadState(); }}
        >{BIN_LABEL[b]}</button>
      {/each}
    </div>
    {#if activeStack}
      <span class="ws" title={activeDir}>{stackLabel(activeStack)}</span>
    {/if}
    <span class="spacer"></span>
    <button class="iconbtn" onclick={discover} title="Re-scan repo for stacks">
      <Icon name="refresh" size={13} />
    </button>
  </div>

  {#if activeStack}
    <div class="actions">
      <button class="act" disabled={!!running} onclick={() => run("init", "init")}>
        {running === "init" ? "Init…" : "Init"}
      </button>
      <button class="act" disabled={!!running} onclick={() => run("validate", "validate")}>
        {running === "validate" ? "Validate…" : "Validate"}
      </button>
      <button class="act primary" disabled={!!running} onclick={() => run("plan", "plan")}>
        {running === "plan" ? "Planning…" : "Plan"}
      </button>
      <span class="spacer"></span>
      <button class="act ext" onclick={() => sendTerminal("apply")} title="Run apply in terminal">Apply ↗</button>
      <button class="act ext danger" onclick={() => sendTerminal("destroy")} title="Run destroy in terminal">Destroy ↗</button>
    </div>
  {/if}

  {#if err}
    <div class="err">{err.includes("not found in PATH") ? `${BIN_LABEL[bin]} not found in PATH.` : err.slice(0, 240)}</div>
  {/if}

  <div class="body">
    <!-- Stacks (discovered IaC dirs) -->
    <div class="stacks">
      <div class="pane-h"><span>Stacks</span><span class="count">{stacks.length}</span></div>
      <div class="stacks-body">
        {#if scanning}
          <div class="empty">Scanning…</div>
        {:else if stacks.length === 0}
          <div class="empty">
            No Terraform / Terragrunt found in this repo.<br /><br />
            Open a repo that contains <code>*.tf</code> or <code>terragrunt.hcl</code> files.
          </div>
        {:else}
          {#each stacks as s (s.path)}
            <div
              class="stack-row"
              class:on={activeStack === s.path}
              role="button"
              tabindex="0"
              title={s.path}
              onclick={() => selectStack(s.path)}
              onkeydown={(e) => e.key === "Enter" && selectStack(s.path)}
            >
              <span class="stack-name">{stackLabel(s.path)}</span>
              {#if s.terragrunt}<span class="tag tg">TG</span>{:else}<span class="tag tf">TF</span>{/if}
            </div>
          {/each}
        {/if}
      </div>
    </div>

    <!-- State resources -->
    <div class="state">
      <div class="pane-h"><span>Resources</span><span class="count">{resources.length}</span></div>
      <div class="state-body">
        {#if !activeStack}
          <div class="empty">Pick a stack.</div>
        {:else if resources.length === 0}
          <div class="empty">No state yet. Run <b>Init</b> then <b>Plan</b>.</div>
        {:else}
          {#each groups as g (g.module)}
            <div class="mod" onclick={() => toggle(g.module)} role="button" tabindex="0"
              onkeydown={(e) => e.key === "Enter" && toggle(g.module)}>
              <span class="chev" class:open={!collapsed[g.module]}>▸</span>
              <span class="mod-name">{g.module}</span>
              <span class="mod-count">{g.items.length}</span>
            </div>
            {#if !collapsed[g.module]}
              {#each g.items as r (r)}
                <div class="res" title={r}>{r}</div>
              {/each}
            {/if}
          {/each}
        {/if}
      </div>
    </div>

    <!-- Output -->
    <div class="out">
      <div class="pane-h">
        <span>{outKind ? outKind[0].toUpperCase() + outKind.slice(1) : "Output"}</span>
        {#if summary}
          {#if summary.none}
            <span class="badge ok">No changes</span>
          {:else}
            <span class="badge add">+{summary.add}</span>
            <span class="badge chg">~{summary.change}</span>
            <span class="badge del">-{summary.destroy}</span>
          {/if}
        {/if}
      </div>
      {#if running === "plan" || running === "init" || running === "validate"}
        <div class="out-body busy">{running}…</div>
      {:else if !activeStack}
        <div class="out-body empty">Select a stack to run Terraform.</div>
      {:else if !output}
        <div class="out-body empty">Run <b>Plan</b> to preview changes in <code>{stackLabel(activeStack)}</code>.</div>
      {:else}
        <pre class="out-body log" bind:this={outEl}>{#each output.split("\n") as l}<span class="ln {lineClass(l)}">{l}
</span>{/each}</pre>
      {/if}
    </div>
  </div>
</div>

<style>
  .tf { display: flex; flex-direction: column; height: 100%; min-height: 0; }

  .topbar {
    display: flex; align-items: center; gap: 10px; height: 32px; flex: 0 0 auto;
    padding: 0 12px; border-bottom: 1px solid var(--border);
  }
  .seg { display: inline-flex; border: 1px solid var(--border); border-radius: 5px; overflow: hidden; }
  .seg-btn {
    padding: 2px 9px; height: 21px; border: 0; background: transparent;
    color: var(--text3); font-size: 11px; cursor: default; border-right: 1px solid var(--border);
  }
  .seg-btn:last-child { border-right: 0; }
  .seg-btn:hover:not(:disabled) { color: var(--text); }
  .seg-btn.on { background: var(--sel); color: var(--text); }
  .seg-btn:disabled { opacity: 0.35; }
  .ws { color: var(--text2); font-size: 11.5px; font-family: var(--font-mono); }
  .spacer { flex: 1; }
  .iconbtn {
    display: inline-flex; align-items: center; justify-content: center;
    width: 22px; height: 20px; border: 0; border-radius: 5px;
    background: transparent; color: var(--text3); cursor: default;
  }
  .iconbtn:hover { background: var(--sel); color: var(--text); }

  .actions {
    display: flex; align-items: center; gap: 6px; height: 36px; flex: 0 0 auto;
    padding: 0 12px; border-bottom: 1px solid var(--border);
  }
  .act {
    padding: 3px 11px; height: 23px; border: 1px solid var(--border);
    background: var(--panel2); color: var(--text2); border-radius: 5px;
    font-size: 11.5px; cursor: default;
  }
  .act:hover:not(:disabled) { color: var(--text); border-color: var(--text3); }
  .act:disabled { opacity: 0.5; }
  .act.primary { color: var(--text); border-color: var(--text3); }
  .act.ext { font-family: var(--font-mono); font-size: 11px; }
  .act.danger:hover:not(:disabled) { color: var(--red); border-color: var(--red); }

  .err {
    padding: 8px 12px; font-size: 11.5px; color: var(--red);
    border-bottom: 1px solid var(--border); flex: 0 0 auto;
    font-family: var(--font-mono); white-space: pre-wrap;
  }

  .body { flex: 1; min-height: 0; display: flex; overflow: hidden; }

  .pane-h {
    display: flex; align-items: center; gap: 7px; height: 26px; flex: 0 0 auto;
    padding: 0 10px; border-bottom: 1px solid var(--border);
    font-size: 10.5px; font-weight: 500; color: var(--text3);
    text-transform: uppercase; letter-spacing: 0.04em;
  }
  .count, .mod-count {
    margin-left: auto; font-family: var(--font-mono); font-size: 9.5px;
    color: var(--text3); opacity: 0.7; letter-spacing: 0;
  }

  /* Stacks */
  .stacks {
    flex: 0 0 210px; min-width: 160px; display: flex; flex-direction: column;
    border-right: 1px solid var(--border);
  }
  .stacks-body { flex: 1; overflow-y: auto; }
  .stack-row {
    display: flex; align-items: center; gap: 6px; height: 24px; padding: 0 10px;
    font-size: 11.5px; color: var(--text2); cursor: default;
    border-bottom: 1px solid var(--hairline);
  }
  .stack-row:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }
  .stack-row.on { background: var(--sel); color: var(--text); }
  .stack-name {
    flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis;
    white-space: nowrap; font-family: var(--font-mono); direction: rtl; text-align: left;
  }
  .tag {
    font-family: var(--font-mono); font-size: 8.5px; font-weight: 600; padding: 1px 4px;
    border-radius: 3px; flex: 0 0 auto; letter-spacing: 0.02em;
  }
  .tag.tg { color: var(--accent); background: color-mix(in srgb, var(--accent) 14%, transparent); }
  .tag.tf { color: var(--text3); background: color-mix(in srgb, var(--text) 8%, transparent); }

  /* State */
  .state {
    flex: 0 0 280px; min-width: 200px; display: flex; flex-direction: column;
    border-right: 1px solid var(--border);
  }
  .state-body { flex: 1; overflow-y: auto; }
  .mod {
    display: flex; align-items: center; gap: 6px; height: 22px; padding: 0 10px;
    font-size: 11px; color: var(--text2); cursor: default;
    background: var(--panel); border-bottom: 1px solid var(--hairline);
  }
  .mod:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }
  .chev { font-size: 8px; color: var(--text3); transition: transform 0.1s; display: inline-block; }
  .chev.open { transform: rotate(90deg); }
  .mod-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font-mono); }
  .res {
    padding: 3px 10px 3px 26px; font-size: 11px; font-family: var(--font-mono);
    color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    border-bottom: 1px solid var(--hairline);
  }
  .res:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }

  /* Output */
  .out { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .badge {
    font-family: var(--font-mono); font-size: 10px; padding: 1px 5px; border-radius: 3px;
    letter-spacing: 0; text-transform: none; font-weight: 600;
  }
  .badge.add { color: var(--green); background: color-mix(in srgb, var(--green) 14%, transparent); }
  .badge.chg { color: var(--yellow); background: color-mix(in srgb, var(--yellow) 14%, transparent); }
  .badge.del { color: var(--red); background: color-mix(in srgb, var(--red) 14%, transparent); }
  .badge.ok { color: var(--green); background: color-mix(in srgb, var(--green) 14%, transparent); }
  .out-body {
    flex: 1; min-height: 0; overflow: auto; margin: 0; padding: 10px 12px;
    font-family: var(--font-mono); font-size: 11px; line-height: 1.5; color: var(--text2);
  }
  .out-body.empty, .out-body.busy { color: var(--text3); font-family: var(--font-ui); font-size: 12px; }
  .out-body.log { white-space: pre; background: var(--bg); }
  .ln { display: block; }
  .ln.add { color: var(--green); }
  .ln.del { color: var(--red); }
  .ln.chg { color: var(--yellow); }
  .ln.rec { color: var(--orange, var(--yellow)); }
  .ln.err { color: var(--red); }
  .ln.ok { color: var(--green); }

  .empty { padding: 16px 12px; color: var(--text3); font-size: 12px; line-height: 1.5; }
  .empty code { font-family: var(--font-mono); color: var(--text2); }
</style>
