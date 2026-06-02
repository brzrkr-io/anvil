<script lang="ts">
  let {
    recent = [],
    onOpenRecent,
    onNewTerminal,
    onCommandPalette,
    onNewFile,
    onNewFolder,
    onOpenFile,
    onOpenFolder,
  }: {
    recent?: string[];
    onOpenRecent?: (path: string) => void;
    onNewTerminal?: () => void;
    onCommandPalette?: () => void;
    onNewFile?: () => void;
    onNewFolder?: () => void;
    onOpenFile?: () => void;
    onOpenFolder?: () => void;
  } = $props();

  function basename(p: string): string {
    return p.split("/").filter(Boolean).at(-1) ?? p;
  }

  function dirname(p: string): string {
    const parts = p.split("/").filter(Boolean);
    if (parts.length <= 1) return "/";
    parts.pop();
    return "/" + parts.join("/");
  }

  const shown = $derived(recent.slice(0, 8));
</script>

<div class="welcome">
  <div class="inner">
    <header>
      <h1 class="wordmark">Anvil<span class="dot">.</span></h1>
      <p class="tagline">The AI-native console for 100% of your work.</p>
    </header>

    <section class="start">
      <div class="section-label">Start</div>
      <div class="action-row">
        <button class="pill" onclick={onNewFile}>New File</button>
        <button class="pill" onclick={onNewFolder}>New Folder</button>
        <button class="pill" onclick={onOpenFile}>Open File… <kbd>⌘O</kbd></button>
        <button class="pill" onclick={onOpenFolder}>Open Folder…</button>
      </div>
      <div class="action-row">
        <button class="pill ghost" onclick={onNewTerminal}>New Terminal <kbd>⌘T</kbd></button>
        <button class="pill ghost" onclick={onCommandPalette}>Command Palette <kbd>⌘K</kbd></button>
      </div>
    </section>

    <section class="recent">
      <div class="section-label">Recent</div>
      {#if shown.length === 0}
        <p class="empty">No recent files yet.</p>
      {:else}
        <ul class="recent-list">
          {#each shown as path (path)}
            <li>
              <button class="recent-item" onclick={() => onOpenRecent?.(path)}>
                <span class="basename">{basename(path)}</span>
                <span class="dirpath">{dirname(path)}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="tips">
      <div class="section-label">Tips</div>
      <ul class="tips-list">
        <li><kbd>⌘P</kbd> <span>Go to file</span></li>
        <li><kbd>⌘E</kbd> <span>Recent files</span></li>
        <li><kbd>⌘⇧F</kbd> <span>Search</span></li>
        <li><kbd>⌘K</kbd> <span>Commands</span></li>
      </ul>
    </section>
  </div>
</div>

<style>
  .welcome {
    height: 100%;
    overflow-y: auto;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg);
    color: var(--text);
    font-family: var(--font-ui);
  }

  .inner {
    width: min(520px, 90vw);
    padding: 48px 0;
    display: flex;
    flex-direction: column;
    gap: 40px;
  }

  header {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .wordmark {
    margin: 0;
    font-size: 42px;
    font-weight: 700;
    letter-spacing: -1px;
    color: var(--text);
    line-height: 1;
  }

  .dot {
    color: var(--accent);
  }

  .tagline {
    margin: 0;
    font-size: 14px;
    color: var(--text3);
  }

  .section-label {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text3);
    margin-bottom: 12px;
  }

  .action-row {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }
  .action-row + .action-row {
    margin-top: 10px;
  }
  /* Primary actions (file/folder) read solid; secondary (terminal/palette) ghost. */
  .pill:not(.ghost) {
    border-color: var(--accent);
    color: var(--text);
  }
  .pill.ghost {
    color: var(--text3);
  }

  .pill {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text2);
    font-family: var(--font-ui);
    font-size: 13px;
    cursor: pointer;
    transition: border-color 0.15s, color 0.15s;
  }

  .pill:hover {
    border-color: var(--accent);
    color: var(--text);
  }

  .pill kbd {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text3);
  }

  .empty {
    margin: 0;
    font-size: 13px;
    color: var(--text3);
  }

  .recent-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .recent-item {
    width: 100%;
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 7px 10px;
    border-radius: 8px;
    border: none;
    background: transparent;
    cursor: pointer;
    text-align: left;
    transition: background 0.12s;
  }

  .recent-item:hover {
    background: var(--panel);
  }

  .basename {
    flex-shrink: 0;
    font-size: 13px;
    color: var(--text);
  }

  .dirpath {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tips-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .tips-list li {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13px;
    color: var(--text2);
  }

  .tips-list kbd {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text3);
    min-width: 44px;
  }
</style>
