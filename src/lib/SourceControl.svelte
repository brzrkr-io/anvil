<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { parseLog, parseStatus, parseConventional, relTime, buildGraph, buildFileTree, type Commit, type Change, type FileNode } from "$lib/git";
  import Icon from "$lib/Icon.svelte";
  import HunkStage from "$lib/HunkStage.svelte";
  import { llmCreds } from "$lib/accounts";
  import { askText } from "$lib/dialog";

  let { cwd, onOpenDiff }: {
    cwd: string;
    onOpenDiff?: (t: { path?: string; staged?: boolean; rev?: string }) => void;
  } = $props();

  let branch = $state("");
  let commits = $state<Commit[]>([]);
  let commitStats = $state<Record<string, { add: number; del: number }>>({});
  let changes = $state<Change[]>([]);
  let error = $state("");
  let sel = $state(0);
  let commitMsg = $state("");
  let amend = $state(false);
  let aheadBehind = $state<{ a: number; b: number } | null>(null);
  let repoFeatures = $state<string[]>([]);
  let busy = $state(false);
  let branches = $state<{ name: string; cur: boolean }[]>([]);
  let stashes = $state<string[]>([]);
  let tags = $state<string[]>([]);
  const now = Date.now();

  const staged = $derived(changes.filter((c) => c.staged));
  const unstaged = $derived(changes.filter((c) => !c.staged));
  const stagedTree = $derived(buildFileTree(staged));
  const unstagedTree = $derived(buildFileTree(unstaged));
  let collapsed = $state<Set<string>>(new Set());
  function toggleDir(p: string) {
    const s = new Set(collapsed);
    s.has(p) ? s.delete(p) : s.add(p);
    collapsed = s;
  }

  // Inline per-hunk staging (#62): keyed by `${staged}:${path}`.
  let expandedHunks = $state<Set<string>>(new Set());
  function toggleHunks(key: string) {
    const s = new Set(expandedHunks);
    s.has(key) ? s.delete(key) : s.add(key);
    expandedHunks = s;
  }

  // Commit-graph swimlanes.
  const LANE = 12, ROW_H = 22, NODE_R = 3;
  const graph = $derived(buildGraph(commits));
  const graphW = $derived(Math.max(1, ...graph.map((r) => r.width)) * LANE);
  const LANE_COLORS = ["--accent", "--green", "--blue", "--purple", "--teal", "--yellow", "--accent2", "--red"];
  const laneColor = (i: number) => `var(${LANE_COLORS[i % LANE_COLORS.length]})`;
  const laneX = (c: number) => c * LANE + LANE / 2;
  function segPaths(row: ReturnType<typeof buildGraph>[number]): { d: string; color: number }[] {
    const out: { d: string; color: number }[] = [];
    const cx = laneX(row.col);
    const mid = ROW_H / 2;
    const c = (x1: number, y1: number, x2: number, y2: number) => {
      const my = (y1 + y2) / 2;
      return `M${x1} ${y1} C ${x1} ${my} ${x2} ${my} ${x2} ${y2}`;
    };
    for (const s of row.segments) {
      const fx = laneX(s.fromCol), tx = laneX(s.toCol);
      if (!s.node) { out.push({ d: c(fx, 0, tx, ROW_H), color: s.color }); continue; }
      if (s.toCol === row.col) out.push({ d: c(fx, 0, cx, mid), color: s.color }); // incoming
      if (s.fromCol === row.col) out.push({ d: c(cx, mid, tx, ROW_H), color: s.color }); // outgoing
    }
    return out;
  }

  // Virtualize the commit log (#93): render only rows in view + overscan so a
  // long history stays smooth. Mirrors SearchPanel's windowing.
  let logViewport = $state<HTMLElement>();
  let logScrollTop = $state(0);
  let logViewH = $state(600);
  const OVERSCAN = 8;
  const visStart = $derived(Math.max(0, Math.floor(logScrollTop / ROW_H) - OVERSCAN));
  const visEnd = $derived(Math.min(commits.length, Math.ceil((logScrollTop + logViewH) / ROW_H) + OVERSCAN));
  function onLogScroll() {
    if (logViewport) { logScrollTop = logViewport.scrollTop; logViewH = logViewport.clientHeight; }
  }

  // Commit popover (click a history row → card with meta + changed files;
  // click a file → that file's diff at the commit).
  let popover = $state<{ commit: Commit; x: number; y: number; files: { code: string; path: string }[] } | null>(null);
  async function openCommitPopover(c: Commit, ev: MouseEvent) {
    sel = commits.indexOf(c);
    let files: { code: string; path: string }[] = [];
    try {
      const raw = await invoke<string>("git_commit_files", { cwd, rev: c.short });
      files = raw.split("\n").filter(Boolean).map((l) => {
        const p = l.split("\t");
        return { code: p[0][0] ?? "M", path: p[p.length - 1] };
      });
    } catch { /* ignore */ }
    // Anchor the popover at the click point (Terax-style).
    const x = Math.min(ev.clientX + 6, window.innerWidth - 420);
    const y = Math.min(ev.clientY + 6, window.innerHeight - 332);
    popover = { commit: c, x: Math.max(8, x), y: Math.max(8, y), files };
  }
  function openFileAt(rev: string, path: string) {
    popover = null;
    onOpenDiff?.({ rev, path });
  }
  function copySha(sha: string) { try { navigator.clipboard?.writeText(sha); } catch (e) { console.warn("clipboard write failed", e); } }
  function fullDate(ts: number): string {
    return new Date(ts * 1000).toLocaleString("en-US", { month: "short", day: "numeric", year: "numeric", hour: "numeric", minute: "2-digit" });
  }

  async function act(cmd: string, args: Record<string, unknown>) {
    busy = true;
    try { await invoke(cmd, { cwd, ...args }); } catch (e) { error = String(e); }
    await load();
    busy = false;
  }
  const stage = (path: string) => act("git_stage", { path });
  const unstage = (path: string) => act("git_unstage", { path });
  const stageAll = () => act("git_stage_all", {});
  // Commit (Terax/Zed-style): auto-stage everything if nothing is staged (so a
  // bare "Commit" just works), optional amend, optional push after.
  async function commit(push = false) {
    const msg = commitMsg.trim();
    if (!msg && !amend) { error = "Write a commit message first"; return; }
    busy = true;
    error = "";
    try {
      if (!staged.length && !amend) await invoke("git_stage_all", { cwd });
      if (amend && !msg) {
        await invoke("git_amend", { cwd });
      } else {
        await invoke("git_commit", { cwd, message: msg, amend });
      }
      if (push) await invoke("git_push", { cwd });
      commitMsg = "";
      amend = false;
    } catch (e) {
      error = String(e);
    }
    await load();
    busy = false;
  }
  async function toggleAmend() {
    amend = !amend;
    if (amend && !commitMsg.trim()) {
      try { commitMsg = (await invoke<string>("git_last_message", { cwd })).replace(/\s+$/, ""); } catch (e) { console.warn("git_last_message failed", e); }
    }
  }
  function onCommitKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") { e.preventDefault(); commit(true); }
  }
  const pull = () => act("git_pull", {});
  const push = () => act("git_push", {});
  // Co-author trailer picker (#30) from the repo's recent commit authors.
  const coAuthors = $derived([...new Set(commits.map((c) => `${c.author} <${c.email}>`))].slice(0, 40));
  function addCoAuthor(a: string) {
    if (!a) return;
    const trailer = `Co-authored-by: ${a}`;
    if (commitMsg.includes(trailer)) return;
    commitMsg = commitMsg.replace(/\s*$/, "") + `\n\n${trailer}`;
  }
  // #30 Saved commit message templates (persisted).
  function loadTemplates(): string[] { try { return JSON.parse(localStorage.getItem("anvil-commit-templates") || "[]"); } catch { return []; } }
  let templates = $state<string[]>(typeof localStorage !== "undefined" ? loadTemplates() : []);
  function persistTemplates() { if (typeof localStorage !== "undefined") localStorage.setItem("anvil-commit-templates", JSON.stringify(templates)); }
  function saveTemplate() { const t = commitMsg.trim(); if (!t) return; templates = [...templates.filter((x) => x !== t), t].slice(-20); persistTemplates(); }
  let tplOpen = $state(false);
  let moreOpen = $state(false);

  // #42 Agent-written commit message from the staged diff (in-place).
  let genBusy = $state(false);
  async function genMessage() {
    if (genBusy) return;
    genBusy = true;
    try {
      let diff = await invoke<string>("git_diff", { cwd, path: ".", staged: true });
      if (!diff.trim()) { await invoke("git_stage_all", { cwd }); diff = await invoke<string>("git_diff", { cwd, path: ".", staged: true }); }
      if (!diff.trim()) { error = "Nothing to summarize — make some changes first"; return; }
      const { base, apiKey } = await llmCreds();
      const models = await invoke<string[]>("llm_models", { base, apiKey }).catch(() => [] as string[]);
      // #41 Route utility tasks (commit messages) to a configured "fast" model
      // when set and available, else fall back to the first model.
      const util = typeof localStorage !== "undefined" ? localStorage.getItem("anvil-util-model") || "" : "";
      const model = (util && models.includes(util) ? util : models[0]) ?? "";
      const prompt = `Write a Conventional Commits message for this staged diff. One short imperative subject line (<72 chars), then a blank line, then 1-3 concise bullet points if useful. Output ONLY the message.\n\n${diff.slice(0, 12000)}`;
      const reply = await invoke<string>("llm_chat", { model, messages: [{ role: "user", content: prompt }], base, apiKey });
      const msg = reply.replace(/^```[\w]*\n?|```$/g, "").trim();
      if (msg) commitMsg = msg;
    } catch (e) { error = `Message generation failed: ${e}`; }
    finally { genBusy = false; await load(); }
  }
  const stashSave = () => act("git_stash_save", { message: "WIP" });
  const stashApply = (i: number) => act("git_stash_apply", { index: `stash@{${i}}` });
  async function stashWithMessage() {
    const m = await askText({ title: "Stash", value: "WIP" });
    if (m === null) return;
    act("git_stash_push", { message: m || null });
  }
  const stashUntracked = () => act("git_stash_push", { message: "WIP (incl. untracked)", untracked: true });
  const stashFile = (path: string) => act("git_stash_push", { message: `WIP ${path.split("/").pop()}`, paths: [path] });

  const typeColor: Record<string, string> = {
    feat: "var(--green)", fix: "var(--accent)", perf: "var(--purple)",
    docs: "var(--text2)", security: "var(--red)", chore: "var(--text2)",
    refactor: "var(--blue)", test: "var(--teal)",
  };

  // Branch-graph filters (#23): author / message-grep / path, applied server-side.
  let fAuthor = $state("");
  let fGrep = $state("");
  let fPath = $state("");
  let filtersOpen = $state(false);
  const filtersActive = $derived(!!(fAuthor.trim() || fGrep.trim() || fPath.trim()));
  function clearFilters() { fAuthor = ""; fGrep = ""; fPath = ""; load(); }

  // Section disclosure state.
  let stashesOpen = $state(false);
  let tagsOpen = $state(false);
  let historyOpen = $state(true);

  async function load() {
    try {
      const log = await invoke<string>("git_log", { cwd, author: fAuthor || null, grep: fGrep || null, path: fPath || null });
      const st = await invoke<string>("git_status", { cwd });
      const br = await invoke<string>("git_branches", { cwd });
      commits = parseLog(log);
      try {
        const raw = await invoke<string>("git_log_stats", { cwd, author: fAuthor || null, grep: fGrep || null, path: fPath || null });
        const out: Record<string, { add: number; del: number }> = {};
        let cur = "";
        for (const line of raw.split("\n")) {
          if (line.startsWith("\x01")) { cur = line.slice(1).trim(); continue; }
          if (!cur) continue;
          const ins = /(\d+) insertion/.exec(line), del = /(\d+) deletion/.exec(line);
          if (ins || del) out[cur] = { add: ins ? +ins[1] : 0, del: del ? +del[1] : 0 };
        }
        commitStats = out;
      } catch { commitStats = {}; }
      const s = parseStatus(st);
      branch = s.branch;
      changes = s.changes;
      branches = br.split("\n").filter(Boolean).map((l) => {
        const [h, n] = l.split("\t");
        return { name: n ?? "", cur: h === "*" };
      });
      const stl = await invoke<string>("git_stash_list", { cwd });
      stashes = stl.split("\n").filter(Boolean);
      const tg = await invoke<string>("git_tags", { cwd });
      tags = tg.split("\n").filter(Boolean).slice(0, 30);
      try {
        const ab = (await invoke<string>("git_ahead_behind", { cwd })).trim().split(/\s+/).map(Number);
        aheadBehind = (ab[0] || ab[1]) ? { a: ab[0] || 0, b: ab[1] || 0 } : null;
      } catch { aheadBehind = null; }
      try { repoFeatures = (await invoke<string>("git_repo_features", { cwd })).split(",").filter(Boolean); } catch { repoFeatures = []; }
      if (!commits.length && !branch) error = "Not a git repository";
    } catch (e) {
      error = String(e);
    }
  }
  onMount(load);

  function badge(code: string) {
    return code === "A" ? "var(--green)" : code === "D" ? "var(--red)"
      : code === "?" ? "var(--text3)" : "var(--yellow)";
  }
</script>

<div class="scm">
  <div class="head">
    <span class="accent hd-ic"><Icon name="branch" size={13} /></span>
    {#if branches.length}
      <select class="branchsel" value={branch} onchange={(e) => { const v = (e.currentTarget as HTMLSelectElement).value; if (v !== branch) act("git_checkout", { branch: v }); }}>
        {#each branches as b (b.name)}<option value={b.name}>{b.name}</option>{/each}
      </select>
    {:else}
      <span class="accent">{branch || "—"}</span>
    {/if}
    {#each repoFeatures as f (f)}<button class="rfeat" title={f === "submodules" ? "git submodule update --init --recursive" : "git lfs pull"} disabled={busy} onclick={() => act(f === "submodules" ? "git_submodule_update" : "git_lfs_pull", {})}>{f}</button>{/each}
    <span class="sync">
      <button class="syncbtn" class:on={filtersActive || filtersOpen} title="Filter commits (author / message / path)" onclick={() => (filtersOpen = !filtersOpen)}><Icon name="search" size={13} /></button>
      {#if aheadBehind}<span class="ab" title="ahead / behind upstream">↑{aheadBehind.a} ↓{aheadBehind.b}</span>{/if}
      <button class="syncbtn" title="Pull (ff-only)" disabled={busy} onclick={pull}><Icon name="refresh" size={13} /></button>
      <button class="syncbtn" title="Push" disabled={busy} onclick={push}><Icon name="up" size={13} /></button>
    </span>
  </div>

  {#if filtersOpen}
    <div class="logfilters">
      <input placeholder="author" bind:value={fAuthor} onkeydown={(e) => e.key === "Enter" && load()} spellcheck="false" />
      <input placeholder="message contains" bind:value={fGrep} onkeydown={(e) => e.key === "Enter" && load()} spellcheck="false" />
      <input placeholder="path" bind:value={fPath} onkeydown={(e) => e.key === "Enter" && load()} spellcheck="false" />
      <button class="syncbtn" title="Apply" onclick={() => load()}><Icon name="check" size={13} /></button>
      {#if filtersActive}<button class="syncbtn" title="Clear filters" onclick={clearFilters}><Icon name="close" size={13} /></button>{/if}
    </div>
  {/if}

  {#if error}
    <div class="empty">{error}</div>
  {:else}
    {#snippet tree(nodes: FileNode[], isStaged: boolean, depth: number)}
      {#each nodes as n (n.path)}
        {#if n.dir}
          <div class="chg dir" style="padding-left:{14 + depth * 12}px" onclick={() => toggleDir(n.path)} role="button" tabindex="0">
            <svg class="caret {collapsed.has(n.path) ? '' : 'open'}" viewBox="0 0 16 16" aria-hidden="true">
              <path d="M6 4l4 4-4 4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
            <span class="dirname">{n.name}</span>
          </div>
          {#if !collapsed.has(n.path)}{@render tree(n.children, isStaged, depth + 1)}{/if}
        {:else if n.change}
          {@const hk = `${isStaged}:${n.change.path}`}
          <div class="chg" style="padding-left:{14 + depth * 12}px">
            <span class="sdot" style="background:{badge(n.change.code)}" title={n.change.code}></span>
            <span class="fname" onclick={() => onOpenDiff?.({ path: n.change!.path, staged: isStaged })} role="button" tabindex="0">{n.name}</span>
            {#if n.change.path.includes("/")}<span class="fdir">{n.change.path.split("/").slice(0, -1).join("/")}</span>{/if}
            {#if n.change.code !== "?"}
              <button class="op hk {expandedHunks.has(hk) ? 'on' : ''}" title="Stage by hunk" disabled={busy} onclick={() => toggleHunks(hk)}><Icon name="density" size={13} /></button>
            {/if}
            <button class="op" title="Stash just this file" disabled={busy} onclick={() => stashFile(n.change!.path)}><Icon name="stash" size={12} /></button>
            <button class="stagetoggle {isStaged ? 'on' : ''}" title={isStaged ? "Unstage" : "Stage"} disabled={busy} aria-label={isStaged ? "Unstage" : "Stage"} onclick={() => (isStaged ? unstage(n.change!.path) : stage(n.change!.path))}></button>
          </div>
          {#if expandedHunks.has(hk) && n.change.code !== "?"}
            <HunkStage {cwd} path={n.change.path} staged={isStaged} onChanged={load} />
          {/if}
        {/if}
      {/each}
    {/snippet}

    {#if staged.length}
      <div class="sect">Staged <span class="cnt">{staged.length}</span></div>
      <div class="changes">{@render tree(stagedTree, true, 0)}</div>
    {/if}

    {#if unstaged.length}
      <div class="sect">Changes <span class="cnt">{unstaged.length}</span>
        <button class="link" disabled={busy} onclick={stageAll}>Stage all</button>
      </div>
      <div class="changes">{@render tree(unstagedTree, false, 0)}</div>
    {/if}

    {#if stashes.length}
      <div class="sect disclosure" onclick={() => (stashesOpen = !stashesOpen)} role="button" tabindex="0">
        <svg class="caret {stashesOpen ? 'open' : ''}" viewBox="0 0 16 16" aria-hidden="true">
          <path d="M6 4l4 4-4 4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
        Stashes <span class="cnt">{stashes.length}</span>
      </div>
      {#if stashesOpen}
        <div class="changes">
          {#each stashes as s, i (s)}
            <div class="chg"><span class="path" style="margin-right:auto;cursor:default" onclick={() => onOpenDiff?.({ rev: `stash@{${i}}` })} role="button" tabindex="0">{s}</span><button class="op" title="Show diff" disabled={busy} onclick={() => onOpenDiff?.({ rev: `stash@{${i}}` })}><Icon name="search" size={12} /></button><button class="op" title="Apply" disabled={busy} onclick={() => stashApply(i)}><Icon name="stash" size={13} /></button></div>
          {/each}
        </div>
      {/if}
    {/if}

    {#if tags.length}
      <div class="sect disclosure" onclick={() => (tagsOpen = !tagsOpen)} role="button" tabindex="0">
        <svg class="caret {tagsOpen ? 'open' : ''}" viewBox="0 0 16 16" aria-hidden="true">
          <path d="M6 4l4 4-4 4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
        Tags <span class="cnt">{tags.length}</span>
      </div>
      {#if tagsOpen}
        <div class="changes">
          {#each tags as t (t)}
            <div class="chg"><span class="bdg" style="color:var(--green);display:inline-flex"><Icon name="tag" size={12} /></span><span class="path">{t}</span></div>
          {/each}
        </div>
      {/if}
    {/if}

    <div class="sect disclosure" onclick={() => (historyOpen = !historyOpen)} role="button" tabindex="0">
      <svg class="caret {historyOpen ? 'open' : ''}" viewBox="0 0 16 16" aria-hidden="true">
        <path d="M6 4l4 4-4 4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
      History
    </div>
    {#if historyOpen}
      <div class="log" bind:this={logViewport} onscroll={onLogScroll}>
        <div class="logspace" style="height:{commits.length * ROW_H}px">
          {#each commits.slice(visStart, visEnd) as c, ii (c.hash)}
            {@const i = visStart + ii}
            {@const cv = parseConventional(c.subject)}
            {@const g = graph[i]}
            {@const stat = commitStats[c.short]}
            <div class="row {i === sel ? 'sel' : ''}" style="top:{i * ROW_H}px" onclick={(e) => openCommitPopover(c, e)} role="button" tabindex="0">
              <svg class="graph" width={graphW} height={ROW_H} style="flex:0 0 {graphW}px">
                {#each segPaths(g) as p}
                  <path d={p.d} stroke={laneColor(p.color)} fill="none" stroke-width="1.6" />
                {/each}
                <circle cx={laneX(g.col)} cy={ROW_H / 2} r={NODE_R} fill={laneColor(g.color)}
                  stroke="var(--bg)" stroke-width="1.5" />
              </svg>
              <span class="sha mono">{c.short}</span>
              <span class="subj">
                {#if i === 0 && c.refs}<span class="ref">{c.refs.split(",")[0].replace("HEAD -> ", "")}</span>{/if}
                {#if cv}<span class="ctype" style="color:{typeColor[cv.kind] || 'var(--blue)'}">{cv.kind}{cv.scope ? `(${cv.scope})` : ""}:</span> {cv.rest}{:else}{c.subject}{/if}
              </span>
              <span class="auth">{c.author}</span>
              <span class="when mono">{relTime(c.ts, now)}</span>
              <span class="cstat">{#if stat?.add}<span class="cadd">+{stat.add}</span>{/if}{#if stat?.del}<span class="cdel">−{stat.del}</span>{/if}</span>
            </div>
          {/each}
        </div>
      </div>
    {/if}
  {/if}

  {#if branch && !error}
    <footer class="composer">
      <div class="ci-card">
        <textarea
          bind:value={commitMsg}
          onkeydown={onCommitKey}
          placeholder="Commit message"
          rows="2"
          spellcheck="false"
        ></textarea>
        <button class="genmsg" title="Write a commit message from the staged diff (agent)" disabled={genBusy} onclick={genMessage}>{genBusy ? "…" : "gen"}</button>
      </div>
      <div class="ci-actions">
        <button class="cbtn primary big" title="Commit all changes and push (⌘↩)" disabled={busy || (!commitMsg.trim() && !amend)} onclick={() => commit(true)}>
          <Icon name="up" size={12} /> {amend ? "Amend & Push" : "Commit & Push"}{#if aheadBehind?.a}<span class="cbtn-n">{aheadBehind.a + 1}</span>{/if}
        </button>
        <button class="cbtn amend-toggle" class:on={amend} title="Amend last commit" disabled={busy} onclick={toggleAmend}>Amend</button>
        <button class="more {moreOpen ? 'on' : ''}" title="More commit actions" onclick={() => (moreOpen = !moreOpen)} aria-label="More actions">⋯</button>
        {#if moreOpen}
          <div class="mscrim" onclick={() => (moreOpen = false)} role="presentation"></div>
          <div class="moremenu">
            <button disabled={busy || (!commitMsg.trim() && !amend)} onclick={() => { commit(false); moreOpen = false; }}>Commit only (no push)</button>
            <button disabled={busy} onclick={() => { push(); moreOpen = false; }}><Icon name="up" size={12} /> Push only</button>
            <div class="mm-sep"></div>
            <button onclick={() => { tplOpen = !tplOpen; moreOpen = false; }}>Templates…</button>
            {#if coAuthors.length}
              <div class="mm-label">Co-author</div>
              {#each coAuthors as a (a)}<button class="mm-sub" onclick={() => { addCoAuthor(a); moreOpen = false; }}>{a}</button>{/each}
            {/if}
            {#if changes.length}
              <div class="mm-sep"></div>
              <div class="mm-label">Stash</div>
              <button class="mm-sub" disabled={busy} onclick={() => { stashSave(); moreOpen = false; }}>All changes</button>
              <button class="mm-sub" disabled={busy} onclick={() => { stashWithMessage(); moreOpen = false; }}>With message…</button>
              <button class="mm-sub" disabled={busy} onclick={() => { stashUntracked(); moreOpen = false; }}>Including untracked</button>
            {/if}
          </div>
        {/if}
        {#if tplOpen}
          <div class="mscrim" onclick={() => (tplOpen = false)} role="presentation"></div>
          <div class="tplmenu">
            <button onclick={() => { saveTemplate(); tplOpen = false; }}>+ Save current as template</button>
            {#each templates as t (t)}
              <div class="tplrow">
                <button class="tpluse" title={t} onclick={() => { commitMsg = t; tplOpen = false; }}>{t.split("\n")[0]}</button>
                <button class="tplx" title="Delete" onclick={() => { templates = templates.filter((x) => x !== t); persistTemplates(); }}>×</button>
              </div>
            {/each}
            {#if !templates.length}<div class="tplempty">No templates yet</div>{/if}
          </div>
        {/if}
      </div>
    </footer>
  {/if}

  {#if popover}
    <div class="pscrim" onclick={() => (popover = null)} role="presentation"></div>
    <div class="pop" style="left:{popover.x}px;top:{popover.y}px">
      <div class="pmeta">
        <div class="ptop"><span class="psha mono">{popover.commit.short}</span><span class="psub">{popover.commit.subject}</span></div>
        <div class="pauth">{popover.commit.author} · {popover.commit.email} · {fullDate(popover.commit.ts)}</div>
        <button class="pcopy" onclick={() => copySha(popover!.commit.hash)}><Icon name="paperclip" size={12} /> Copy SHA</button>
      </div>
      <div class="pfileshd">FILES <span class="pcnt">{popover.files.length}</span></div>
      <div class="pfiles">
        {#each popover.files as f (f.path)}
          <button class="pfile" onclick={() => openFileAt(popover!.commit.short, f.path)}>
            <span class="pfic"><Icon name="folder" size={12} /></span>
            <span class="pfname mono">{f.path.split("/").pop()}</span>
            <span class="pfdir">{f.path.split("/").slice(0, -1).join("/")}</span>
            <span class="pfbdg" style="color:{badge(f.code)}">{f.code}</span>
          </button>
        {/each}
        {#if !popover.files.length}<div class="pempty">No file changes</div>{/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .scm { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--bg); }
  .pscrim { position: fixed; inset: 0; z-index: 40; }
  .pop { position: fixed; z-index: 41; width: 400px; max-height: 320px; display: flex; flex-direction: column;
    background: var(--glass); backdrop-filter: blur(var(--blur)) saturate(1.3); -webkit-backdrop-filter: blur(var(--blur)) saturate(1.3);
    border: 1px solid var(--border); border-radius: 8px; overflow: hidden;
    box-shadow: var(--elev-3), inset 0 1px 0 var(--hairline); }
  .pmeta { padding: 12px 14px; }
  .ptop { display: flex; align-items: baseline; gap: 8px; }
  .psha { font-size: 11px; color: var(--text3); background: var(--panel2); padding: 1px 6px; border-radius: 6px; flex: 0 0 auto; }
  .psub { font-size: 13px; font-weight: 600; color: var(--text); }
  .pauth { margin-top: 7px; font-size: 11.5px; color: var(--text3); }
  .pcopy { margin-top: 9px; display: inline-flex; align-items: center; gap: 5px; border: 0; background: transparent;
    color: var(--accent); font-size: 11.5px; cursor: default; padding: 0; }
  .pfileshd { display: flex; align-items: center; padding: 7px 14px; border-top: 1px solid var(--border);
    font-size: 10px; letter-spacing: .08em; text-transform: uppercase; font-weight: 600; color: var(--text3); }
  .pcnt { margin-left: auto; color: var(--text3); }
  .pfiles { overflow-y: auto; padding-bottom: 6px; }
  .pfile { display: flex; align-items: center; gap: 8px; width: 100%; padding: 4px 14px; border: 0;
    background: transparent; cursor: default; text-align: left; }
  .pfile:hover { background: var(--panel2); }
  .pfic { display: inline-flex; color: var(--blue); flex: 0 0 auto; }
  .pfname { font-size: 12.5px; color: var(--text); flex: 0 0 auto; }
  .pfdir { font-size: 11px; color: var(--text3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .pfbdg { margin-left: auto; font-family: var(--font-mono); font-weight: 700; font-size: 11px; flex: 0 0 auto; }
  .pempty { padding: 8px 14px; color: var(--text3); font-size: 11.5px; }
  .head { height: 28px; flex: 0 0 auto; display: flex; align-items: center; gap: 10px; padding: 0 12px;
    border-bottom: 1px solid var(--border); font-size: 11.5px; }
  .accent { color: var(--accent); font-weight: 600; }
  .sect { padding: 4px 14px 2px; font-size: 11px; color: var(--text3); font-weight: 500; }
  .sect.disclosure { display: flex; align-items: center; gap: 4px; cursor: default; }
  .changes { padding-bottom: 4px; }
  .chg { display: flex; align-items: center; height: 22px; padding: 0 8px 0 14px; gap: 6px; font-size: 12px;
    transition: background 0.1s ease; }
  .chg:hover { background: color-mix(in srgb, var(--text) 6%, transparent); }
  .chg.dir { cursor: default; }
  .caret { width: 14px; height: 14px; flex: 0 0 auto; color: var(--text3);
    transition: transform 0.12s ease; }
  .caret.open { transform: rotate(90deg); }
  .chg.dir:hover .caret { color: var(--text2); }
  .dirname { color: var(--text2); font-family: var(--font-ui); font-size: 12px; }
  .bdg { width: 14px; text-align: center; font-weight: 700; font-family: var(--font-mono); font-size: 11px; }
  .path { color: var(--text); font-family: var(--font-mono); font-size: 12px; }
  .sdot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; }
  .fname { color: var(--text); font-family: var(--font-mono); font-size: 12px; flex: 0 0 auto; white-space: nowrap;
    overflow: hidden; text-overflow: ellipsis; max-width: 60%; }
  .fname:hover { color: var(--accent); }
  .fdir { flex: 1; min-width: 0; margin-left: 1px; color: var(--text3); font-size: 10.5px; font-family: var(--font-mono);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .stagetoggle { flex: 0 0 auto; width: 14px; height: 14px; border: 1.5px solid var(--text3); border-radius: 3px;
    background: transparent; cursor: default; position: relative; transition: background .1s ease, border-color .1s ease; }
  .chg:hover .stagetoggle { border-color: var(--accent); }
  .stagetoggle.on { background: var(--accent); border-color: var(--accent); }
  .stagetoggle.on::after { content: ""; position: absolute; left: 4px; top: 1px; width: 3.5px; height: 7px;
    border: solid var(--bg); border-width: 0 1.6px 1.6px 0; transform: rotate(45deg); }
  .stagetoggle:disabled { opacity: 0.5; }
  .chg .op { display: none; width: 18px; height: 18px; border: 0; border-radius: 4px;
    background: transparent; color: var(--text3); font-size: 14px; cursor: default;
    align-items: center; justify-content: center; }
  .chg:hover .op { display: inline-flex; background: var(--sel); color: var(--text); }
  .chg .op.hk { margin-left: auto; }
  .chg .op.hk + .op { margin-left: 0; }
  .chg .op.hk.on { color: var(--accent); }
  .log { flex: 1; overflow-y: auto; position: relative; }
  .logspace { position: relative; width: 100%; }
  .row { position: absolute; left: 0; right: 0; display: flex; align-items: center; height: 22px; padding: 0 14px; gap: 0; cursor: default; }
  .row:hover { background: color-mix(in srgb, var(--text) 6%, transparent); }
  .row.sel { background: color-mix(in srgb, var(--accent) 14%, transparent); }
  .graph { flex: 0 0 auto; height: 22px; overflow: visible; margin-right: 6px; }
  .sha { width: 54px; flex: 0 0 auto; color: var(--text3); font-size: 10.5px; }
  .subj { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; color: var(--text); font-size: 12px; }
  .ctype { font-family: var(--font-mono); }
  .ref { display: inline-block; font-size: 9.5px; padding: 0 5px; margin-right: 6px; border-radius: 4px;
    background: var(--accent); color: var(--bg); font-family: var(--font-mono); }
  .auth { width: 64px; flex: 0 0 auto; color: var(--text3); font-size: 10.5px; white-space: nowrap;
    overflow: hidden; text-overflow: ellipsis; }
  .when { width: 40px; flex: 0 0 auto; text-align: right; color: var(--text3); font-size: 10.5px; }
  .cstat { width: 66px; flex: 0 0 auto; text-align: right; font-family: var(--font-mono); font-size: 10px;
    margin-left: 10px; white-space: nowrap; }
  .cadd { color: var(--green); }
  .cdel { color: var(--red); margin-left: 5px; }
  .empty { padding: 24px 14px; color: var(--text3); }
  .cnt { margin-left: 4px; color: var(--text3); }
  .link { margin-left: auto; border: 0; background: transparent; color: var(--accent);
    font-size: 11px; cursor: default; }
  /* Bottom-docked commit composer (Terax-style): slim card pinned to the panel
     foot; secondary actions fold into the ⋯ menu so the footer stays small. */
  .composer { flex: 0 0 auto; display: flex; flex-direction: column; gap: 6px;
    padding: 8px 10px 10px; border-top: 1px solid var(--border); background: var(--panel); position: relative; }
  .ci-card { position: relative; }
  .composer textarea { width: 100%; box-sizing: border-box; resize: none; min-height: 40px; max-height: 160px;
    padding: 6px 28px 6px 8px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg);
    color: var(--text); font-family: var(--font-ui); font-size: 12.5px; line-height: 1.45; outline: 0; }
  .composer textarea:focus { border-color: var(--accent); }
  .ci-actions { display: flex; align-items: center; height: 28px; gap: 6px; position: relative; }
  .cbtn { display: inline-flex; align-items: center; justify-content: center; gap: 5px;
    border: 1px solid var(--border); border-radius: 6px; background: var(--panel2);
    color: var(--text); font-size: 12px; font-weight: 500; cursor: default;
    transition: background .1s ease, border-color .1s ease, opacity .1s ease; }
  .cbtn:hover:not(:disabled) { border-color: var(--accent); }
  .cbtn:disabled { opacity: 0.45; }
  .cbtn.primary { flex: 0 0 auto; height: 24px; padding: 0 12px; border-radius: 6px; font-size: 12px; font-weight: 600;
    border-color: transparent; background: var(--accent); color: var(--bg); }
  .cbtn.primary:hover:not(:disabled) { filter: brightness(1.05); }
  .cbtn.primary.big { flex: 1; height: 26px; }
  .cbtn-n { margin-left: 2px; font-size: 10px; font-weight: 700; opacity: 0.85;
    background: color-mix(in srgb, var(--bg) 28%, transparent); border-radius: 8px; padding: 0 5px; }
  .cbtn.amend-toggle { height: 26px; padding: 0 9px; border-radius: 6px; border: 1px solid var(--border);
    font-size: 11.5px; color: var(--text3); background: transparent; }
  .cbtn.amend-toggle.on { border-color: var(--accent); color: var(--accent); }
  .more { flex: 0 0 auto; width: 28px; height: 24px; display: inline-flex; align-items: center; justify-content: center;
    border: 1px solid var(--border); border-radius: 6px; background: transparent; color: var(--text2);
    font-size: 16px; line-height: 1; cursor: default; transition: border-color .1s ease, color .1s ease; }
  .more:hover, .more.on { border-color: var(--accent); color: var(--text); }
  .mscrim { position: fixed; inset: 0; z-index: 44; }
  .moremenu { position: absolute; bottom: 38px; right: 0; z-index: 45; min-width: 204px; padding: 4px;
    background: var(--glass); backdrop-filter: var(--frost); -webkit-backdrop-filter: var(--frost);
    border: 1px solid var(--border); border-radius: 8px; box-shadow: var(--elev-3), inset 0 1px 0 var(--hairline);
    display: flex; flex-direction: column; gap: 1px; }
  .moremenu > button { display: flex; align-items: center; gap: 6px; text-align: left; border: 0; background: transparent;
    color: var(--text); font-family: var(--font-ui); font-size: 12px; padding: 6px 9px; border-radius: 6px; cursor: default; }
  .moremenu > button:hover:not(:disabled) { background: var(--sel); }
  .moremenu > button:disabled { opacity: 0.45; }
  .moremenu .mm-sub { padding-left: 16px; color: var(--text2); font-size: 11.5px; }
  .mm-label { padding: 5px 9px 2px; font-size: 9.5px; letter-spacing: .07em; text-transform: uppercase;
    font-weight: 600; color: var(--text3); }
  .mm-sep { height: 1px; margin: 4px 6px; background: var(--border); }
  .rfeat { font-size: 9.5px; text-transform: uppercase; letter-spacing: .05em; color: var(--text3);
    border: 1px solid var(--border); border-radius: 5px; padding: 1px 5px; background: transparent; cursor: default; }
  .rfeat:hover:not(:disabled) { color: var(--text); border-color: var(--accent); }
  .rfeat:disabled { opacity: 0.5; }
  .sync { margin-left: auto; display: flex; align-items: center; gap: 6px; }
  .sync .ab { color: var(--text3); font-family: var(--font-mono); font-size: 11px; }
  .syncbtn { display: inline-flex; align-items: center; justify-content: center; width: 22px; height: 20px;
    border: 0; border-radius: 5px; background: transparent; color: var(--text3); cursor: default; }
  .syncbtn:hover:not(:disabled) { background: var(--sel); color: var(--text); }
  .syncbtn:disabled { opacity: 0.4; }
  .syncbtn.on { color: var(--accent); background: color-mix(in srgb, var(--accent) 14%, transparent); }
  .logfilters { display: flex; align-items: center; gap: 6px; padding: 6px 14px; border-bottom: 1px solid var(--border); }
  .logfilters input { flex: 1; min-width: 0; background: var(--bg); color: var(--text); border: 1px solid var(--border);
    border-radius: 5px; padding: 3px 7px; font-size: 11.5px; font-family: var(--font-mono); outline: 0; }
  .logfilters input:focus { border-color: var(--accent); }
  .genmsg { position: absolute; top: 6px; right: 6px; border: 1px solid var(--border); background: var(--panel2);
    color: var(--text2); border-radius: 4px; width: 18px; height: 18px; font-size: 10px; font-family: var(--font-mono);
    cursor: default; display: inline-flex; align-items: center; justify-content: center; padding: 0; }
  .genmsg:hover:not(:disabled) { border-color: var(--accent); color: var(--text); }
  .genmsg:disabled { opacity: 0.5; }
  .tplmenu { position: absolute; bottom: 38px; right: 0; z-index: 45; min-width: 220px; max-height: 320px; overflow-y: auto;
    background: var(--glass); backdrop-filter: var(--frost); -webkit-backdrop-filter: var(--frost);
    border: 1px solid var(--border); border-radius: 8px; padding: 4px; box-shadow: var(--elev-3), inset 0 1px 0 var(--hairline); }
  .tplmenu > button, .tplrow .tpluse { display: block; width: 100%; text-align: left; border: 0; background: transparent; color: var(--text);
    font-family: var(--font-ui); font-size: 12px; padding: 5px 8px; border-radius: 5px; cursor: default; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tplmenu > button:hover, .tplrow .tpluse:hover { background: var(--sel); }
  .tplrow { display: flex; align-items: center; }
  .tplrow .tpluse { flex: 1; min-width: 0; }
  .tplx { border: 0; background: transparent; color: var(--text3); cursor: default; padding: 0 6px; }
  .tplx:hover { color: var(--text); }
  .tplempty { color: var(--text3); font-size: 11px; padding: 6px 8px; }
  .branchsel { border: 0; background: transparent; appearance: none; padding: 0 2px;
    font-size: 12px; font-weight: 600; color: var(--accent); border-radius: 6px; outline: 0; cursor: default; }
  .head .hd-ic { display: inline-flex; align-items: center; }
</style>
