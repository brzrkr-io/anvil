<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import TreeNode from "./TreeNode.svelte";
  import FileIcon from "./FileIcon.svelte";

  interface Entry { name: string; path: string; is_dir: boolean; }

  let {
    entry,
    depth = 0,
    onOpenFile,
    onReload,
  }: {
    entry: Entry;
    depth?: number;
    onOpenFile?: (p: string) => void;
    onReload?: () => void;
  } = $props();

  let expanded = $state(false);
  let children = $state<Entry[]>([]);
  let loaded = $state(false);
  // #73 Cap how many children render at once so a huge folder can't blow up the
  // DOM; "… N more" reveals the rest in chunks.
  const MORE_STEP = 300;
  let shown = $state(MORE_STEP);

  let menu = $state<{ x: number; y: number } | null>(null);

  async function loadChildren() {
    try { children = await invoke<Entry[]>("list_dir", { path: entry.path }); }
    catch (e) { children = []; console.warn("list_dir failed", e); }
    shown = MORE_STEP;
    loaded = true;
  }

  async function toggle() {
    if (!entry.is_dir) { onOpenFile?.(entry.path); return; }
    if (!loaded) await loadChildren();
    expanded = !expanded;
  }

  function openMenu(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    menu = { x: e.clientX, y: e.clientY };
  }

  function closeMenu() { menu = null; }

  function parentDir(p: string) {
    const i = p.replace(/\/$/, "").lastIndexOf("/");
    return i > 0 ? p.slice(0, i) : p;
  }

  async function newFile() {
    closeMenu();
    const dir = entry.is_dir ? entry.path : parentDir(entry.path);
    const name = prompt("New file name:");
    if (!name) return;
    await invoke("create_path", { path: dir + "/" + name, isDir: false });
    await refresh(dir);
  }

  async function newFolder() {
    closeMenu();
    const dir = entry.is_dir ? entry.path : parentDir(entry.path);
    const name = prompt("New folder name:");
    if (!name) return;
    await invoke("create_path", { path: dir + "/" + name, isDir: true });
    await refresh(dir);
  }

  async function rename() {
    closeMenu();
    const newName = prompt("Rename to:", entry.name);
    if (!newName || newName === entry.name) return;
    const dir = parentDir(entry.path);
    await invoke("rename_path", { from: entry.path, to: dir + "/" + newName });
    onReload?.();
  }

  async function del() {
    closeMenu();
    if (!confirm("Delete " + entry.name + "?")) return;
    await invoke("delete_path", { path: entry.path });
    onReload?.();
  }

  async function refresh(dir: string) {
    if (entry.is_dir && entry.path === dir) {
      await loadChildren();
      expanded = true;
    } else {
      onReload?.();
    }
  }

</script>

<div
  class="row"
  role="button"
  tabindex="0"
  style="padding-left: calc(8px + {depth * 12}px)"
  onclick={toggle}
  onkeydown={(e) => e.key === "Enter" && toggle()}
  oncontextmenu={openMenu}
>
  {#if entry.is_dir}
    <svg class="chev {expanded ? 'open' : ''}" viewBox="0 0 16 16" aria-hidden="true">
      <path d="M6 4l4 4-4 4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
    <FileIcon dir open={expanded} />
    <span class="nm dir">{entry.name}</span>
  {:else}
    <span class="chev-sp"></span>
    <FileIcon name={entry.name} />
    <span class="nm">{entry.name}</span>
  {/if}
</div>

{#if menu}
  <div class="scrim" onclick={closeMenu} role="presentation"></div>
  <div class="ctx-menu" style="left:{menu.x}px;top:{menu.y}px">
    {#if entry.is_dir}
      <button onclick={newFile}>New File</button>
      <button onclick={newFolder}>New Folder</button>
      <hr />
    {/if}
    <button onclick={rename}>Rename</button>
    <button class="danger" onclick={del}>Delete</button>
  </div>
{/if}

{#if expanded}
  {#each children.slice(0, shown) as child, i (child.path + '#' + i)}
    <TreeNode entry={child} depth={depth + 1} {onOpenFile} onReload={loadChildren} />
  {/each}
  {#if children.length > shown}
    <button class="more" style="padding-left:{12 + depth * 14}px" onclick={() => (shown += MORE_STEP)}>
      … {children.length - shown} more
    </button>
  {/if}
{/if}

<style>
  .row {
    display: flex; align-items: center; gap: 6px; padding-right: 12px; height: var(--row-h, 25px);
    font-size: var(--ui-fs, 13px); color: var(--text); cursor: default;
  }
  .row:hover { background: var(--panel2); }
  .more { display: block; width: 100%; text-align: left; border: 0; background: transparent; color: var(--text3);
    font-family: var(--font-ui); font-size: 11px; padding: 3px 8px; cursor: default; }
  .more:hover { color: var(--accent); }
  .chev { width: 14px; height: 14px; flex: 0 0 auto; color: var(--text3);
    transition: transform 0.12s ease; }
  .chev.open { transform: rotate(90deg); }
  .row:hover .chev { color: var(--text2); }
  .chev-sp { width: 14px; flex: 0 0 auto; }
  .nm { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .nm.dir { color: var(--text); font-weight: 500; }

  .scrim {
    position: fixed; inset: 0; z-index: 999;
  }
  .ctx-menu {
    position: fixed; z-index: 1000;
    background: var(--panel2); border: 1px solid var(--border);
    border-radius: 6px; padding: 4px 0; min-width: 140px;
    box-shadow: 0 4px 16px rgba(0,0,0,0.35);
    font-family: var(--font-ui); font-size: 12.5px;
  }
  .ctx-menu button {
    display: block; width: 100%; padding: 5px 14px; text-align: left;
    background: none; border: none; color: var(--text); cursor: default;
    font-size: 12.5px; font-family: var(--font-ui);
  }
  .ctx-menu button:hover { background: var(--sel); }
  .ctx-menu button.danger { color: var(--failure, #e06a3b); }
  .ctx-menu hr { border: none; border-top: 1px solid var(--border); margin: 3px 0; }
</style>
