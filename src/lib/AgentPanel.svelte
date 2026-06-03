<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, Channel } from "@tauri-apps/api/core";
  import { planSteps } from "$lib/agent";
  import { get } from "svelte/store";
  import { agentQueue, dequeueAgent } from "$lib/agent-queue";
  import { online } from "$lib/offline";
  import { applyRedaction as redact, auditAgentSend } from "$lib/redaction";
  import { llmCreds } from "$lib/accounts";
  import { agentSeed, agentInvestigate } from "$lib/agent-seed";
  import Icon from "$lib/Icon.svelte";
  import DiffReview from "$lib/DiffReview.svelte";
  import { parseToolCalls, toolResultMessage, parseEditBlocks, riskyCommand, TOOL_SYSTEM_PROMPT, type ToolCall } from "$lib/agent-tools";
  import { listRuns, archiveRun, loadRun, deleteRun, type RunMeta, type RunMsg } from "$lib/agent-history";

  let {
    cwd,
    onRunCommand,
    attachPath = "",
    listFiles,
    onReadFile,
    onApplyFile,
    onReply,
    getTerminalText,
  }: {
    cwd: string;
    onRunCommand?: (cmd: string) => void;
    attachPath?: string;
    listFiles?: () => Promise<string[]>;
    onReadFile?: (path: string) => Promise<string>;
    onApplyFile?: (path: string, content: string) => void;
    onReply?: (summary: string) => void;
    getTerminalText?: () => string;
  } = $props();

  let attached = $state(false);
  let termAttached = $state(false);
  let repoAttached = $state(false);
  const base = (p: string) => p.split("/").pop() ?? p;

  interface Message { role: "user" | "assistant"; text: string; }

  // Split assistant text into prose + fenced code blocks so shell suggestions
  // get a one-click (approval-gated) Run button.
  interface Seg { code: boolean; lang: string; text: string; }
  function parseBlocks(t: string): Seg[] {
    const segs: Seg[] = [];
    const re = /```(\w*)\n?([\s\S]*?)```/g;
    let last = 0;
    let m: RegExpExecArray | null;
    while ((m = re.exec(t))) {
      if (m.index > last) segs.push({ code: false, lang: "", text: t.slice(last, m.index) });
      segs.push({ code: true, lang: m[1] || "", text: m[2] });
      last = re.lastIndex;
    }
    if (last < t.length) segs.push({ code: false, lang: "", text: t.slice(last) });
    return segs;
  }
  const RUNNABLE = new Set(["bash", "sh", "shell", "zsh", ""]);

  // Plan checklist (#49): a ```plan fenced block renders as toggleable steps.
  let planDone = $state<Record<string, boolean>>({});

  // Inline per-hunk diff review (#54): open the agent's proposed file content as
  // an accept/reject-per-hunk diff against the current file instead of a blind
  // whole-file overwrite.
  let review = $state<{ path: string; proposed: string; current: string } | null>(null);
  async function openReview(path: string, proposed: string) {
    let current = "";
    try { current = onReadFile ? await onReadFile(path) : ""; } catch { current = ""; }
    review = { path, proposed, current };
  }

  let messages = $state<Message[]>([]);
  let input = $state("");
  // Pre-fill from an external "explain this" action (#55).
  $effect(() => {
    const s = $agentSeed;
    if (s) { input = s; agentSeed.set(""); }
  });
  // Agent-driven investigation: seed, turn on Agent (tool-use) mode, auto-send.
  $effect(() => {
    const s = $agentInvestigate;
    if (s) {
      agentInvestigate.set("");
      autoTools = true;
      input = s;
      send();
    }
  });
  let models = $state<string[]>([]);
  let model = $state("");
  let busy = $state(false);

  // Rough token meter (~4 chars/token) over the conversation.
  const tokens = $derived(Math.round(messages.reduce((n, m) => n + m.text.length, 0) / 4));

  // #40 Per-request meter + session cost / budget. Costs are user-configured
  // ($/1k tokens, 0 = off); the budget warns when the session estimate exceeds it.
  function lnum(k: string): number { if (typeof localStorage === "undefined") return 0; const n = Number(localStorage.getItem(k)); return Number.isFinite(n) ? n : 0; }
  let lastIn = $state(0);
  let lastOut = $state(0);
  let sessionTok = $state(0);
  let costPer1k = $state(lnum("anvil-agent-cost"));
  let budget = $state(lnum("anvil-agent-budget"));
  const sessionCost = $derived(costPer1k > 0 ? (sessionTok / 1000) * costPer1k : 0);
  const overBudget = $derived(budget > 0 && sessionCost > budget);
  function setCost(v: number) { costPer1k = Math.max(0, v); if (typeof localStorage !== "undefined") localStorage.setItem("anvil-agent-cost", String(costPer1k)); }
  function setBudget(v: number) { budget = Math.max(0, v); if (typeof localStorage !== "undefined") localStorage.setItem("anvil-agent-budget", String(budget)); }

  // @-mention: type "@" then a query to attach a workspace file as context.
  let fileList = $state<string[]>([]);
  let mentionQuery = $state<string | null>(null);
  let mentionSel = $state(0);
  let ta: HTMLTextAreaElement | undefined;
  const mentionHits = $derived(
    mentionQuery === null ? []
      : fileList.filter((f) => f.toLowerCase().includes(mentionQuery!.toLowerCase())).slice(0, 8)
  );
  function onInput() {
    const pos = ta?.selectionStart ?? input.length;
    const m = input.slice(0, pos).match(/@([^\s@]*)$/);
    mentionQuery = m ? m[1] : null;
    mentionSel = 0;
  }
  function pickMention(rel: string) {
    const pos = ta?.selectionStart ?? input.length;
    const before = input.slice(0, pos).replace(/@([^\s@]*)$/, `@${rel} `);
    input = before + input.slice(pos);
    mentionQuery = null;
    ta?.focus();
  }

  // Cap retained chat so a long-running session can't grow memory / localStorage
  // unbounded (and so the payload sent to the model stays bounded).
  const MSG_CAP = 400;

  onMount(async () => {
    const saved = typeof localStorage !== "undefined" ? localStorage.getItem("anvil-agent-chat") : null;
    if (saved) { try { messages = (JSON.parse(saved) as Message[]).slice(-MSG_CAP); } catch { /* ignore */ } }
    try {
      const { base, apiKey } = await llmCreds();
      models = await invoke<string[]>("llm_models", { base, apiKey });
      // #41 Model router: prefer the configured reasoning model for agent chat.
      const reasoning = typeof localStorage !== "undefined" ? localStorage.getItem("anvil-reasoning-model") || "" : "";
      model = (reasoning && models.includes(reasoning) ? reasoning : models[0]) ?? "";
    } catch { models = []; }
    if (listFiles) { try { fileList = await listFiles(); } catch { fileList = []; } }
  });

  // Trim to the cap whenever the history grows past it (no-op once at the cap, so
  // it can't loop). Keeps the most recent exchanges.
  $effect(() => { if (messages.length > MSG_CAP) messages = messages.slice(-MSG_CAP); });

  // Persist conversation across view switches.
  $effect(() => {
    if (typeof localStorage !== "undefined") localStorage.setItem("anvil-agent-chat", JSON.stringify(messages));
  });

  // Auto-follow the streaming response: keep the transcript pinned to the bottom
  // as tokens arrive, unless the user has scrolled up to read history. A
  // "jump to latest" affordance reappears when they're not at the bottom.
  let msgsEl = $state<HTMLElement>();
  let stick = $state(true);
  function onMsgsScroll() {
    if (!msgsEl) return;
    stick = msgsEl.scrollHeight - msgsEl.scrollTop - msgsEl.clientHeight < 48;
  }
  function jumpToLatest() {
    stick = true;
    if (msgsEl) msgsEl.scrollTop = msgsEl.scrollHeight;
  }
  $effect(() => {
    // Re-run when a message is appended (length) AND while the last message
    // streams in (its text grows) — both are reactive reads.
    messages.length;
    const last = messages[messages.length - 1];
    void last?.text;
    if (stick && msgsEl) {
      const el = msgsEl;
      requestAnimationFrame(() => { el.scrollTop = el.scrollHeight; });
    }
  });
  // #8 Archive the finished conversation as a reopenable run before clearing.
  let runs = $state<RunMeta[]>(listRuns());
  let historyOpen = $state(false);
  function newChat() {
    if (messages.length) { archiveRun(messages as RunMsg[], String(Date.now()), Date.now()); runs = listRuns(); }
    messages = [];
  }
  function openRun(id: string) {
    if (messages.length) { archiveRun(messages as RunMsg[], String(Date.now()), Date.now()); }
    messages = loadRun(id) as Message[];
    runs = listRuns();
    historyOpen = false;
  }
  function removeRun(id: string) { runs = deleteRun(id); }
  function fmtAgo(ts: number): string {
    const s = Math.max(0, Math.floor((Date.now() - ts) / 1000));
    if (s < 60) return "just now";
    if (s < 3600) return `${Math.floor(s / 60)}m ago`;
    if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
    return `${Math.floor(s / 86400)}d ago`;
  }

  async function send() {
    const text = input.trim();
    if (!text || busy) return;
    if (!get(online)) { messages = [...messages, { role: "user", text }, { role: "assistant", text: "You're offline — reconnect to use the agent." }]; input = ""; return; } // #79
    messages = [...messages, { role: "user", text }];
    input = "";
    if (!model) {
      messages = [...messages, { role: "assistant", text: "No local model found. Load a model in LM Studio (or any OpenAI-compatible server on localhost:1234)." }];
      return;
    }
    const ctx: { role: string; content: string }[] = [];
    if (repoAttached && listFiles) {
      try {
        const files = await listFiles();
        if (files.length) ctx.push({ role: "user", content: `Project file map (${cwd}):\n\`\`\`\n${files.slice(0, 600).join("\n")}\n\`\`\`` });
        // #33 Import graph so the agent understands module dependencies.
        try {
          const graph = await invoke<string>("repo_import_graph", { root: cwd });
          if (graph.trim()) ctx.push({ role: "user", content: `Module import graph (file: import):\n\`\`\`\n${graph.slice(0, 6000)}\n\`\`\`` });
        } catch { /* rg missing */ }
      } catch { /* ignore */ }
    }
    if (termAttached && getTerminalText) {
      const t = redact(getTerminalText());
      if (t.trim()) ctx.push({ role: "user", content: `Recent terminal output:\n\`\`\`\n${t.slice(-6000)}\n\`\`\`` });
    }
    if (attached && attachPath && onReadFile) {
      try {
        const fc = await onReadFile(attachPath);
        ctx.push({ role: "user", content: `Attached file ${attachPath}:\n\`\`\`\n${redact(fc)}\n\`\`\`` });
      } catch { /* ignore */ }
    }
    // Resolve @-mentions: @path → file, @sym:Name → symbol matches, @<sha> → commit (#38).
    const seen = new Set<string>();
    for (const mm of text.matchAll(/@([^\s@]+)/g)) {
      const rel = mm[1];
      if (seen.has(rel)) continue;
      seen.add(rel);
      if (onReadFile && fileList.includes(rel)) {
        try {
          const fc = await onReadFile(`${cwd.replace(/\/$/, "")}/${rel}`);
          ctx.push({ role: "user", content: `Mentioned file ${rel}:\n\`\`\`\n${redact(fc)}\n\`\`\`` });
        } catch { /* ignore */ }
      } else if (rel.startsWith("sym:") && cwd) {
        const sym = rel.slice(4);
        if (sym) {
          try {
            const hits = await invoke<string>("grep", { root: cwd, query: `\\b${sym.replace(/[^\w]/g, "")}\\b` });
            const lines = hits.split("\n").filter(Boolean).slice(0, 30).map((l) => l.replace(`${cwd.replace(/\/$/, "")}/`, ""));
            if (lines.length) ctx.push({ role: "user", content: `Symbol "${sym}" occurrences:\n\`\`\`\n${redact(lines.join("\n"))}\n\`\`\`` });
          } catch { /* rg missing */ }
        }
      } else if (/^[0-9a-f]{7,40}$/i.test(rel) && cwd) {
        try {
          const show = await invoke<string>("git_show", { cwd, rev: rel });
          if (show.trim()) ctx.push({ role: "user", content: `Mentioned commit ${rel}:\n\`\`\`\n${redact(show).slice(0, 8000)}\n\`\`\`` });
        } catch { /* ignore */ }
      }
    }
    toolSteps = 0;
    auditAgentSend("agent.send", [text, ...ctx.map((c) => c.content)].join("\n"));
    await runTurn(ctx);
  }

  // One model round-trip against the current message history (+ optional ctx).
  // Returns the assistant text and, in agent mode, surfaces any tool call.
  // #39 Per-project agent rules: fold AGENTS.md / CLAUDE.md / .cursorrules from
  // the workspace into the system prompt (cached per cwd).
  let rulesCwd = "";
  let projectRules = "";
  async function loadProjectRules() {
    if (rulesCwd === cwd) return;
    rulesCwd = cwd; projectRules = "";
    if (!onReadFile || !cwd) return;
    for (const f of ["AGENTS.md", "CLAUDE.md", ".cursorrules"]) {
      try { const c = await onReadFile(`${cwd.replace(/\/$/, "")}/${f}`); if (c && c.trim()) { projectRules = `${f}:\n${c.slice(0, 4000)}`; break; } } catch { /* ignore */ }
    }
  }

  async function runTurn(ctx: { role: string; content: string }[] = []) {
    busy = true;
    pendingTool = null;
    await loadProjectRules();
    let base = `You are the AI agent inside Anvil, a developer console. The working directory is ${cwd}. Be concise and practical. When asked to edit the attached file, reply with the COMPLETE new file contents in a single fenced code block. For a multi-step task, FIRST output a fenced \`\`\`plan code block with one short step per line, then the details.`;
    if (projectRules) base += `\n\nProject conventions (follow these):\n${projectRules}`;
    if (autoTools) {
      base += "\n\n" + TOOL_SYSTEM_PROMPT;
      base += "\n\nOperational context: this is a DevOps console. Where present on PATH you may use kubectl, flux, terragrunt, terraform, helm, aws, gh, glab, and docker via the run tool. Always investigate before mutating — read state first (get / describe / status / plan / logs). For Kubernetes managed by FluxCD, prefer `flux` (get / reconcile / logs / suspend) over `kubectl apply`, since the cluster is reconciled from git.";
    }
    const sys = { role: "system", content: base };
    const hist = messages.map((m) => ({ role: m.role, content: m.text }));
    messages = [...messages, { role: "assistant", text: "…" }];
    const ai = messages.length - 1;
    const payload = { model, messages: [sys, ...ctx, ...hist] };
    lastIn = Math.round(payload.messages.reduce((n, m) => n + (m.content?.length ?? 0), 0) / 4);
    let acc = "";
    try {
      const { base: apiBase, apiKey } = await llmCreds();
      const onToken = new Channel<string>();
      onToken.onmessage = (tok) => { if (!acc) messages[ai].text = ""; acc += tok; messages[ai].text = acc; };
      try {
        await invoke("llm_chat_stream", { ...payload, base: apiBase, apiKey, onToken });
      } catch {
        const reply = await invoke<string>("llm_chat", { ...payload, base: apiBase, apiKey });
        acc = reply;
        messages[ai].text = reply;
      }
      if (!acc) messages[ai].text = "(empty response)";
      lastOut = Math.round(acc.length / 4);
      sessionTok += lastIn + lastOut;
      onReply?.(acc ? acc.replace(/```[\s\S]*?```/g, "").trim().slice(0, 80) || "response ready" : "response ready");
    } catch (e) {
      messages[ai].text = `Error: ${e}. Is a model loaded in LM Studio on localhost:1234?`;
    }
    busy = false;
    if (autoTools) {
      const calls = parseToolCalls(acc);
      if (calls.length && toolSteps < MAX_TOOL_STEPS) pendingTool = calls[0];
    }
    // #37 Auto-drain the background queue when idle (no pending tool call).
    if (!pendingTool && get(agentQueue).length) {
      const next = dequeueAgent();
      if (next) { input = next.prompt; send(); }
    }
  }

  // Approval-gated tool execution + loop continuation (#53).
  const MAX_TOOL_STEPS = 12; // room for diagnose → fix → verify (and one iterate)
  let autoTools = $state(false);
  let toolSteps = $state(0);
  let pendingTool = $state<ToolCall | null>(null);

  async function approveTool() {
    const call = pendingTool;
    if (!call || busy) return;
    pendingTool = null;
    let output = "";
    try {
      if (call.kind === "run") {
        output = await invoke<string>("run_capture", { cwd, command: call.arg });
      } else {
        const p = call.arg.startsWith("/") ? call.arg : `${cwd.replace(/\/$/, "")}/${call.arg}`;
        output = onReadFile ? redact(await onReadFile(p)) : "(no file access)";
      }
    } catch (e) {
      output = `error: ${e}`;
    }
    messages = [...messages, { role: "user", text: toolResultMessage(call, output) }];
    toolSteps += 1;
    await runTurn([]);
  }
  function rejectTool() { pendingTool = null; }

  function onKeydown(e: KeyboardEvent) {
    if (mentionQuery !== null && mentionHits.length) {
      if (e.key === "ArrowDown") { e.preventDefault(); mentionSel = (mentionSel + 1) % mentionHits.length; return; }
      if (e.key === "ArrowUp") { e.preventDefault(); mentionSel = (mentionSel - 1 + mentionHits.length) % mentionHits.length; return; }
      if (e.key === "Enter" || e.key === "Tab") { e.preventDefault(); pickMention(mentionHits[mentionSel]); return; }
      if (e.key === "Escape") { e.preventDefault(); mentionQuery = null; return; }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  }

  const CHIPS = ["Explain this error", "Write a commit message", "Summarize recent commits"];
</script>

<div class="ap">
  <div class="header">
    <span class="title">Agent {busy ? "· thinking…" : ""}</span>
    {#if tokens > 0}<span class="meter" title="Approx. tokens in conversation">~{tokens > 999 ? (tokens / 1000).toFixed(1) + "k" : tokens} tok</span>{/if}
    {#if lastIn > 0}<span class="meter" title="Last request: input / output tokens (est.)">{lastIn}→{lastOut}</span>{/if}
    {#if costPer1k > 0}<span class="meter" class:over={overBudget} title="Estimated session cost / budget">${sessionCost.toFixed(3)}{#if budget > 0}/${budget.toFixed(2)}{/if}</span>{/if}
    <button class="newchat" title="Set $ / 1k tokens and a session budget cap" onclick={() => { const c = prompt('Cost $ per 1k tokens (0 = off):', String(costPer1k)); if (c !== null) setCost(Number(c) || 0); const b = prompt('Session budget cap in $ (0 = off):', String(budget)); if (b !== null) setBudget(Number(b) || 0); }}>$</button>
    {#if runs.length}
      <button class="newchat" onclick={() => (historyOpen = !historyOpen)} title="Past agent runs"><Icon name="history" size={13} /></button>
    {/if}
    {#if messages.length}<button class="newchat" onclick={newChat} title="New chat (archives this one)"><Icon name="plus" size={13} /> New</button>{/if}
    {#if historyOpen}
      <div class="ap-histscrim" role="presentation" onclick={() => (historyOpen = false)}></div>
      <div class="ap-hist">
        <div class="ap-hist-h">Past runs</div>
        {#each runs as r (r.id)}
          <div class="ap-hist-row">
            <button class="ap-hist-open" onclick={() => openRun(r.id)} title={r.title}>
              <span class="ap-hist-title">{r.title}</span>
              <span class="ap-hist-ago">{fmtAgo(r.ts)}</span>
            </button>
            <button class="ap-hist-x" onclick={() => removeRun(r.id)} title="Delete run"><Icon name="close" size={11} /></button>
          </div>
        {/each}
      </div>
    {/if}
    <button class="newchat agentmode {autoTools ? 'on' : ''}" onclick={() => (autoTools = !autoTools)} title="Agent mode: the agent runs commands & reads files via approval-gated tool calls">Agent {autoTools ? "on" : "off"}</button>
    <select bind:value={model} class="picker">
      {#if models.length === 0}<option value="">No local model</option>{/if}
      {#each models as m (m)}<option value={m}>{m}</option>{/each}
    </select>
  </div>

  <div class="msgs" bind:this={msgsEl} onscroll={onMsgsScroll}>
    {#if messages.length === 0}
      <div class="empty">
        <p class="hint">Ask the agent anything about your workspace.</p>
        <div class="chips">
          {#each CHIPS as chip}
            <button class="chip" onclick={() => { input = chip; }}>
              {chip}
            </button>
          {/each}
        </div>
      </div>
    {:else}
      {#each messages as msg (msg)}
        <div class="msg-row {msg.role}">
          <div class="msg-label">{msg.role === "assistant" ? "agent" : "you"}</div>
          <div class="msg-body">
            {#if msg.role === "assistant"}
              {#each parseBlocks(msg.text) as seg}
                {#if seg.code && seg.lang.toLowerCase() === "plan"}
                  {@const steps = planSteps(seg.text)}
                  {@const done = steps.filter((s) => planDone[s]).length}
                  <div class="plan">
                    <div class="planhd">Plan <span class="planct">{done}/{steps.length}</span></div>
                    {#each steps as step, si (step)}
                      <button class="pstep" class:done={planDone[step]} onclick={() => (planDone = { ...planDone, [step]: !planDone[step] })}>
                        <span class="pbox">{planDone[step] ? "✓" : si + 1}</span>
                        <span class="ptext">{step}</span>
                      </button>
                    {/each}
                  </div>
                {:else if seg.code}
                  <div class="code">
                    <pre>{seg.text}</pre>
                    {#if RUNNABLE.has(seg.lang.toLowerCase()) && onRunCommand}
                      <button class="run" onclick={() => onRunCommand?.(seg.text.trim())}><Icon name="play" size={12} /> Run in terminal</button>
                    {:else if attachPath && onApplyFile}
                      <button class="run" onclick={() => openReview(attachPath, seg.text)}><Icon name="pencil" size={12} /> Review changes</button>
                      <button class="run ghost" onclick={() => onApplyFile?.(attachPath, seg.text)}>Apply all</button>
                    {/if}
                  </div>
                  {#if review && review.path === attachPath && review.proposed === seg.text}
                    <DiffReview path={review.path} proposed={review.proposed} current={review.current}
                      onApply={(p, merged) => { onApplyFile?.(p, merged); review = null; }}
                      onCancel={() => (review = null)} />
                  {/if}
                {:else}{seg.text}{/if}
              {/each}
              {@const edits = parseEditBlocks(msg.text)}
              {#if edits.length}
                <div class="edits">
                  <div class="editshd">Proposed edits · {edits.length} file{edits.length === 1 ? "" : "s"}</div>
                  {#each edits as ed (ed.path)}
                    {@const full = ed.path.startsWith("/") ? ed.path : `${cwd.replace(/\/$/, "")}/${ed.path}`}
                    <button class="editrow" onclick={() => openReview(full, ed.content)}>
                      <Icon name="pencil" size={12} /> <span class="ep">{ed.path}</span>
                    </button>
                    {#if review && review.path === full && review.proposed === ed.content}
                      <DiffReview path={review.path} proposed={review.proposed} current={review.current}
                        onApply={(p, merged) => { onApplyFile?.(p, merged); review = null; }}
                        onCancel={() => (review = null)} />
                    {/if}
                  {/each}
                </div>
              {/if}
            {:else}{msg.text}{/if}
          </div>
        </div>
      {/each}
    {/if}
  </div>

  {#if pendingTool}
    <div class="toolcard">
      <div class="tchd">
        <span class="tckind">{pendingTool.kind === "run" ? "Run command" : "Read file"}</span>
        <span class="tcstep">step {toolSteps + 1}/{MAX_TOOL_STEPS}</span>
      </div>
      <pre class="tcarg">{pendingTool.arg}</pre>
      {#if pendingTool.kind === "run"}
        {@const risk = riskyCommand(pendingTool.arg)}
        {#if risk}
          <div class="tcrisk"><Icon name="alert" size={12} /> Risk: {risk}. Review carefully before approving.</div>
        {/if}
      {/if}
      <div class="tcacts">
        <button class="tcapprove" disabled={busy} onclick={approveTool}>Approve</button>
        <button class="tcreject" disabled={busy} onclick={rejectTool}>Skip</button>
      </div>
    </div>
  {/if}

  {#if attachPath || getTerminalText || listFiles}
    <div class="attachrow">
      {#if attachPath}
        <button class="attach" class:on={attached} onclick={() => (attached = !attached)} title="Include the active file as context">
          <Icon name="paperclip" size={12} /> {base(attachPath)}{attached ? " · attached" : ""}
        </button>
      {/if}
      {#if getTerminalText}
        <button class="attach" class:on={termAttached} onclick={() => (termAttached = !termAttached)} title="Include recent terminal output as context">
          <Icon name="terminal" size={12} /> terminal{termAttached ? " · attached" : ""}
        </button>
      {/if}
      {#if listFiles}
        <button class="attach" class:on={repoAttached} onclick={() => (repoAttached = !repoAttached)} title="Include the project file map as context">
          <Icon name="folder" size={12} /> repo map{repoAttached ? " · attached" : ""}
        </button>
      {/if}
    </div>
  {/if}

  {#if !stick && messages.length}
    <button class="jump-latest" onclick={jumpToLatest} title="Jump to latest">↓ latest</button>
  {/if}

  <div class="composer">
    {#if mentionQuery !== null && mentionHits.length}
      <div class="mentions">
        {#each mentionHits as f, i (f)}
          <button class="mention {i === mentionSel ? 'on' : ''}" onmousedown={(e) => { e.preventDefault(); pickMention(f); }}>
            <span class="mname">{f.split("/").pop()}</span><span class="mpath">{f}</span>
          </button>
        {/each}
      </div>
    {/if}
    <textarea
      bind:this={ta}
      bind:value={input}
      oninput={onInput}
      onkeydown={onKeydown}
      placeholder="Message the agent… (@ to attach a file)"
      rows={2}
      spellcheck={false}
    ></textarea>
    <button class="send" onclick={send} disabled={!input.trim()}>Send</button>
  </div>
</div>

<style>
  .ap {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    font-family: var(--font-ui);
  }

  .header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--panel);
    flex: 0 0 auto;
    position: relative;
  }
  /* #8 Past-runs dropdown. */
  .ap-histscrim { position: fixed; inset: 0; z-index: 40; }
  .ap-hist { position: absolute; top: 100%; right: 8px; z-index: 41; margin-top: 4px;
    width: 280px; max-height: 320px; overflow-y: auto; background: var(--panel2, var(--panel));
    border: 1px solid var(--border); border-radius: 8px; box-shadow: var(--elev-2, 0 8px 24px rgba(0,0,0,0.4)); padding: 4px; }
  .ap-hist-h { font-size: 10px; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text3); padding: 6px 8px 4px; }
  .ap-hist-row { display: flex; align-items: center; gap: 2px; }
  .ap-hist-open { flex: 1; display: flex; flex-direction: column; gap: 1px; align-items: flex-start; min-width: 0;
    background: none; border: 0; border-radius: 6px; padding: 6px 8px; cursor: default; text-align: left; }
  .ap-hist-open:hover { background: color-mix(in srgb, var(--text) 6%, transparent); }
  .ap-hist-title { font-size: 12px; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 230px; }
  .ap-hist-ago { font-size: 10px; color: var(--text3); }
  .ap-hist-x { background: none; border: 0; color: var(--text3); cursor: default; padding: 4px; display: inline-flex; flex: 0 0 auto; }
  .ap-hist-x:hover { color: var(--status-failure, #e5484d); }

  .title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text2);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    flex: 1;
  }

  .meter {
    font-family: var(--font-mono);
    font-size: 10.5px;
    color: var(--text3);
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-radius: 10px;
    flex: 0 0 auto;
  }
  .meter.over { color: #fff; background: var(--danger, #e5484d); border-color: transparent; }

  .newchat {
    background: var(--panel2);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text2);
    font-family: var(--font-ui);
    font-size: 11.5px;
    padding: 3px 8px;
    cursor: default;
    flex: 0 0 auto;
  }
  .newchat:hover { background: var(--sel); color: var(--text); }

  .picker {
    background: var(--panel2);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    font-family: var(--font-ui);
    font-size: 12px;
    padding: 3px 6px;
    outline: none;
    cursor: default;
  }

  .msgs {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 8px 0;
    display: flex;
    flex-direction: column;
  }

  .empty {
    margin: auto;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }

  .hint {
    color: var(--text3);
    font-size: 12.5px;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    justify-content: center;
  }

  .chip {
    background: var(--panel2);
    border: 1px solid var(--border);
    border-radius: 20px;
    color: var(--text2);
    font-family: var(--font-ui);
    font-size: 11.5px;
    padding: 4px 10px;
    cursor: default;
  }

  .chip:hover {
    background: var(--sel);
    color: var(--text);
  }

  .msg-row {
    display: grid;
    grid-template-columns: 42px 1fr;
    padding: 7px 12px;
    border-bottom: 1px solid var(--hairline);
  }
  .msg-row:last-child { border-bottom: 0; }

  .msg-label {
    font-family: var(--font-mono);
    font-size: 10px;
    line-height: 1.9;
    color: var(--text3);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    user-select: none;
    flex: 0 0 auto;
  }
  .msg-row.assistant .msg-label { color: var(--purple); }

  .msg-body {
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--text);
    white-space: pre-wrap;
    word-break: break-word;
    min-width: 0;
  }

  .code {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    margin: 6px 0;
    overflow: hidden;
  }
  .code pre {
    margin: 0;
    padding: 8px 10px;
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text);
  }
  .run {
    width: 100%;
    border: 0;
    border-top: 1px solid var(--border);
    background: transparent;
    color: var(--accent);
    font-family: var(--font-ui);
    font-size: 11.5px;
    font-weight: 600;
    padding: 5px 0;
    cursor: default;
  }
  .run:hover { background: var(--sel); }
  .run { display: flex; align-items: center; justify-content: center; gap: 5px; }
  .run.ghost { color: var(--text3); font-weight: 500; }
  .edits { margin: 8px 0; border: 1px solid var(--border); border-radius: 9px; overflow: hidden; }
  .edits .editshd { padding: 6px 10px; background: var(--panel); border-bottom: 1px solid var(--border);
    font-size: 11px; font-weight: 600; color: var(--text2); }
  .editrow { display: flex; align-items: center; gap: 7px; width: 100%; padding: 6px 10px; border: 0;
    background: transparent; color: var(--accent); font-size: 12px; cursor: default; text-align: left; }
  .editrow:hover { background: var(--panel2); }
  .editrow .ep { color: var(--text); font-family: var(--font-mono); }
  .agentmode { border: 1px solid var(--border); border-radius: 6px; padding: 2px 7px; background: transparent;
    color: var(--text3); font-size: 11px; cursor: default; }
  .agentmode.on { border-color: var(--status-agent); color: var(--status-agent); }
  .toolcard { margin: 8px 12px; border: 1px solid var(--status-agent); border-radius: 9px; overflow: hidden;
    background: var(--panel); }
  .tchd { display: flex; align-items: center; padding: 6px 10px; border-bottom: 1px solid var(--border); }
  .tckind { flex: 1; font-size: 11.5px; font-weight: 600; color: var(--accent); }
  .tcstep { font-size: 10.5px; color: var(--text3); }
  .tcarg { margin: 0; padding: 8px 10px; font-family: var(--font-mono); font-size: 12px; color: var(--text);
    white-space: pre-wrap; word-break: break-all; max-height: 120px; overflow-y: auto; }
  .tcacts { display: flex; gap: 8px; padding: 8px 10px; border-top: 1px solid var(--border); }
  .tcapprove { border: 0; border-radius: 6px; padding: 4px 12px; background: var(--status-agent); color: var(--bg);
    font-size: 11.5px; font-weight: 600; cursor: default; }
  .tcreject { border: 1px solid var(--border); border-radius: 6px; padding: 4px 12px; background: transparent;
    color: var(--text2); font-size: 11.5px; cursor: default; }
  .tcapprove:disabled, .tcreject:disabled { opacity: 0.5; }
  .tcrisk { display: flex; align-items: center; gap: 6px; padding: 6px 10px; font-size: 11.5px;
    color: var(--risk, #e5484d); background: color-mix(in srgb, var(--risk, #e5484d) 12%, transparent);
    border-top: 1px solid color-mix(in srgb, var(--risk, #e5484d) 35%, transparent); }
  .newchat, .attach { display: inline-flex; align-items: center; gap: 5px; }

  .plan {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    margin: 6px 0;
    overflow: hidden;
  }
  .planhd {
    padding: 5px 10px;
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    font-weight: 700;
    color: var(--text3);
    border-bottom: 1px solid var(--border);
  }
  .planct { color: var(--accent); margin-left: 4px; }
  .pstep {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    width: 100%;
    border: 0;
    border-bottom: 1px solid var(--border);
    background: transparent;
    padding: 6px 10px;
    text-align: left;
    cursor: default;
  }
  .pstep:last-child { border-bottom: 0; }
  .pstep:hover { background: var(--sel); }
  .pbox {
    flex: 0 0 auto;
    width: 18px;
    height: 18px;
    border-radius: 5px;
    border: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text3);
  }
  .pstep.done .pbox { background: var(--green); color: var(--bg); border-color: var(--green); }
  .ptext { color: var(--text); font-size: 12.5px; line-height: 1.4; }
  .pstep.done .ptext { color: var(--text3); text-decoration: line-through; }

  .attachrow { padding: 4px 10px 0; }
  .attach {
    border: 1px solid var(--border); background: var(--panel2); color: var(--text2);
    font-family: var(--font-ui); font-size: 11px; padding: 3px 9px; border-radius: 20px; cursor: default;
  }
  .attach.on { background: var(--accent); color: var(--bg); border-color: var(--accent); }

  .jump-latest {
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
    bottom: 76px;
    z-index: 4;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 12px;
    border: 1px solid var(--border);
    border-radius: 14px;
    background: var(--bg1);
    color: var(--text2);
    font-size: 11px;
    cursor: default;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.3);
  }
  .jump-latest:hover { background: var(--sel); color: var(--text); }

  .composer {
    position: relative;
    display: flex;
    align-items: flex-end;
    gap: 8px;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
    background: var(--panel);
    flex: 0 0 auto;
  }

  .mentions {
    position: absolute;
    left: 10px;
    right: 10px;
    bottom: calc(100% - 4px);
    background: var(--panel2);
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.35);
    z-index: 5;
  }
  .mention {
    display: flex;
    align-items: baseline;
    gap: 8px;
    width: 100%;
    border: 0;
    background: transparent;
    padding: 5px 10px;
    cursor: default;
    text-align: left;
  }
  .mention.on { background: var(--sel); }
  .mname { color: var(--text); font-family: var(--font-mono); font-size: 12px; }
  .mpath { color: var(--text3); font-size: 10.5px; margin-left: auto; overflow: hidden;
    text-overflow: ellipsis; white-space: nowrap; max-width: 60%; }

  .composer textarea {
    flex: 1;
    resize: none;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--text);
    font-family: var(--font-ui);
    font-size: 13px;
    line-height: 1.4;
    outline: none;
    padding: 7px 10px;
  }

  .composer textarea::placeholder {
    color: var(--text3);
  }

  .send {
    background: var(--accent);
    border: none;
    border-radius: 8px;
    color: var(--bg);
    cursor: default;
    font-family: var(--font-ui);
    font-size: 12.5px;
    font-weight: 600;
    padding: 7px 14px;
    flex: 0 0 auto;
  }

  .send:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .send:not(:disabled):hover {
    filter: brightness(1.1);
  }
</style>
