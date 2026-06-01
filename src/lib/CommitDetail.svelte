<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  let { cwd, rev }: { cwd: string; rev: string } = $props();

  interface FileStat {
    path: string;
    ins: number | null;
    del: number | null;
  }

  interface CommitInfo {
    hash: string;
    short: string;
    author: string;
    date: string;
    subject: string;
    body: string;
    files: FileStat[];
  }

  let info = $state<CommitInfo | null>(null);
  let error = $state("");

  function parse(raw: string): CommitInfo {
    const lines = raw.split("\n");
    let i = 0;

    const hashLine = lines[i] ?? "";
    const hash = hashLine.startsWith("commit ") ? hashLine.slice(7).trim() : hashLine.trim();
    const short = hash.slice(0, 8);
    i++;

    // Optional merge line
    if (lines[i]?.startsWith("Merge:")) i++;

    const authorLine = lines[i] ?? "";
    const author = authorLine.startsWith("Author:") ? authorLine.slice(7).trim() : "";
    i++;

    const dateLine = lines[i] ?? "";
    const date = dateLine.startsWith("Date:") ? dateLine.slice(5).trim() : "";
    i++;

    // blank line before message
    if (lines[i] === "") i++;

    // message lines are indented with 4 spaces
    const msgLines: string[] = [];
    while (i < lines.length && (lines[i].startsWith("    ") || lines[i] === "")) {
      msgLines.push(lines[i].slice(4));
      i++;
    }
    // trim trailing blanks
    while (msgLines.length && msgLines[msgLines.length - 1] === "") msgLines.pop();
    const subject = msgLines[0] ?? "";
    const body = msgLines.slice(1).join("\n").trimStart();

    // stat block: lines like " path/file | 12 +++--"
    // they appear after the message and before "diff --git" hunks
    const files: FileStat[] = [];
    const statRe = /^ (.+?)\s+\|\s+(\d+)?\s*([+]*)(-*)/;
    for (; i < lines.length; i++) {
      const l = lines[i];
      if (l.startsWith("diff --git")) break;
      const m = l.match(statRe);
      if (m) {
        files.push({
          path: m[1].trim(),
          ins: m[3] ? m[3].length : null,
          del: m[4] ? m[4].length : null,
        });
      }
    }

    return { hash, short, author, date, subject, body, files };
  }

  async function load() {
    error = "";
    info = null;
    if (!cwd || !rev) return;
    try {
      const raw = await invoke<string>("git_show", { cwd, rev });
      info = parse(raw);
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    void cwd;
    void rev;
    load();
  });

  function initial(name: string): string {
    return (name.trim()[0] ?? "?").toUpperCase();
  }
</script>

<div class="cd">
  {#if error}
    <div class="empty">{error}</div>
  {:else if !info}
    <div class="empty">Loading…</div>
  {:else}
    <div class="header">
      <div class="avatar">{initial(info.author)}</div>
      <div class="meta">
        <span class="author">{info.author}</span>
        <span class="date">{info.date}</span>
      </div>
      <span class="sha mono">{info.short}</span>
    </div>

    <div class="body">
      <div class="subject">{info.subject}</div>
      {#if info.body}
        <pre class="msg-body">{info.body}</pre>
      {/if}

      {#if info.files.length}
        <div class="sect">Files <span class="cnt">{info.files.length}</span></div>
        <div class="files">
          {#each info.files as f (f.path)}
            <div class="file-row">
              <span class="fpath mono">{f.path}</span>
              <span class="counts">
                {#if f.ins !== null}<span class="ins">+{f.ins}</span>{/if}
                {#if f.del !== null}<span class="del">−{f.del}</span>{/if}
              </span>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .cd {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow-y: auto;
    background: var(--bg);
    font-family: var(--font-ui, system-ui, sans-serif);
  }
  .empty {
    padding: 24px 16px;
    color: var(--text3);
    font-size: 13px;
  }
  .header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
    flex: 0 0 auto;
    background: var(--panel);
  }
  .avatar {
    width: 34px;
    height: 34px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--bg);
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    font-size: 15px;
    flex: 0 0 auto;
    font-family: var(--font-ui, system-ui, sans-serif);
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .author {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .date {
    color: var(--text3);
    font-size: 11px;
  }
  .sha {
    flex: 0 0 auto;
    font-size: 11px;
    color: var(--text2);
    background: var(--panel2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 2px 8px;
    letter-spacing: 0.04em;
  }
  .body {
    padding: 14px 16px;
    flex: 1;
    min-height: 0;
  }
  .subject {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
    line-height: 1.4;
    margin-bottom: 8px;
  }
  .msg-body {
    color: var(--text2);
    font-size: 12.5px;
    font-family: var(--font-ui, system-ui, sans-serif);
    white-space: pre-wrap;
    margin: 0 0 16px;
    line-height: 1.6;
  }
  .sect {
    padding: 10px 0 5px;
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    font-weight: 600;
    color: var(--text3);
  }
  .cnt {
    color: var(--text3);
    margin-left: 4px;
  }
  .files {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .file-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 3px 0;
    font-size: 12px;
  }
  .fpath {
    flex: 1;
    min-width: 0;
    color: var(--text2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .counts {
    display: flex;
    gap: 6px;
    flex: 0 0 auto;
    font-family: var(--font-mono, monospace);
    font-size: 11.5px;
  }
  .ins { color: var(--green); }
  .del { color: var(--red); }
  .mono { font-family: var(--font-mono, monospace); }
</style>
