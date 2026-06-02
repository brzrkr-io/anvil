<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import TreeNode from "./TreeNode.svelte";
  import Icon from "./Icon.svelte";
  import { askText } from "$lib/dialog";

  let { path = $bindable(""), onOpenFile }: { path: string; onOpenFile?: (p: string) => void } = $props();

  interface Entry { name: string; path: string; is_dir: boolean; }
  let entries = $state<Entry[]>([]);
  // J92: cap rendered rows so a huge directory can't build thousands of nodes
  // at once; reveal more on demand.
  const PAGE = 300;
  let cap = $state(PAGE);

  let menu = $state<{ x: number; y: number } | null>(null);

  async function load(p: string) {
    try { entries = await invoke<Entry[]>("list_dir", { path: p }); }
    catch (e) { entries = []; console.warn("list_dir failed", e); }
  }
  $effect(() => { if (path) { load(path); cap = PAGE; } });

  function up() {
    const i = path.replace(/\/$/, "").lastIndexOf("/");
    if (i > 0) path = path.slice(0, i);
  }

  function openMenu(e: MouseEvent) {
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY };
  }

  function closeMenu() { menu = null; }

  async function newFile() {
    closeMenu();
    const name = await askText({ title: "New file", placeholder: "name.ext" });
    if (!name) return;
    await invoke("create_path", { path: path + "/" + name, isDir: false });
    await load(path);
  }

  async function newFolder() {
    closeMenu();
    const name = await askText({ title: "New folder" });
    if (!name) return;
    await invoke("create_path", { path: path + "/" + name, isDir: true });
    await load(path);
  }
</script>

<div class="fb" role="tree" tabindex="-1" oncontextmenu={openMenu}>
  <div class="cwd" title={path}>{path.split("/").pop() || "/"}</div>
  <div class="row up" onclick={up} role="button" tabindex="0" onkeydown={(e) => e.key === "Enter" && up()}>
    <span class="ic folder" style="display:inline-flex"><Icon name="up" size={12} /></span><span class="nm">..</span>
  </div>
  {#each entries.slice(0, cap) as e (e.path)}
    <TreeNode entry={e} depth={0} {onOpenFile} onReload={() => load(path)} />
  {/each}
  {#if entries.length > cap}
    <div class="row more" onclick={() => (cap += PAGE)} role="button" tabindex="0" onkeydown={(ev) => ev.key === "Enter" && (cap += PAGE)}>
      <span class="nm">… {entries.length - cap} more</span>
    </div>
  {/if}
</div>

{#if menu}
  <div class="scrim" onclick={closeMenu} role="presentation"></div>
  <div class="ctx-menu" style="left:{menu.x}px;top:{menu.y}px">
    <button onclick={newFile}>New File</button>
    <button onclick={newFolder}>New Folder</button>
  </div>
{/if}

<style>
  .fb { padding-bottom: 8px; }
  .cwd {
    padding: 9px 12px 6px; color: var(--text3); font-size: 10px; font-weight: 500;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .row {
    display: flex; align-items: center; gap: 4px; margin: 0 6px; padding: 4px 6px 4px 2px;
    height: var(--row-h, 25px); border-radius: 6px;
    font-size: var(--ui-fs, 13px); color: var(--text); cursor: default;
    transition: background 0.1s ease;
  }
  .row:hover { background: color-mix(in srgb, var(--text) 6%, transparent); }
  .row.up { color: var(--text3); }
  .row.more { color: var(--accent); font-size: 11.5px; }
  .ic { width: 12px; flex: 0 0 auto; color: var(--text3); font-size: 9px; text-align: center; }
  .ic.folder { color: var(--text3); }
  .nm { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

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
</style>
