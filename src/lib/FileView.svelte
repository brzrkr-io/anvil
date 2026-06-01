<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";

  // Non-text file viewer (#7): images/SVG inline, everything else a placeholder.
  let { path }: { path: string } = $props();
  const ext = $derived(path.split(".").pop()?.toLowerCase() ?? "");
  const isImage = $derived(["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg", "avif"].includes(ext));
  const src = $derived(convertFileSrc(path));
  const name = $derived(path.split("/").pop() ?? path);
</script>

<div class="fv">
  {#if isImage}
    <div class="imgwrap">
      <img {src} alt={name} />
    </div>
    <div class="bar mono">{name}</div>
  {:else}
    <div class="binph">
      <div class="bi">⬡</div>
      <div class="t">{name}</div>
      <div class="s">Binary file — no preview</div>
    </div>
  {/if}
</div>

<style>
  .fv { width: 100%; height: 100%; min-height: 0; display: flex; flex-direction: column; background: var(--bg); }
  .imgwrap {
    flex: 1; min-height: 0; display: flex; align-items: center; justify-content: center; padding: 24px; overflow: auto;
    background-image: linear-gradient(45deg, var(--panel) 25%, transparent 25%), linear-gradient(-45deg, var(--panel) 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, var(--panel) 75%), linear-gradient(-45deg, transparent 75%, var(--panel) 75%);
    background-size: 18px 18px; background-position: 0 0, 0 9px, 9px -9px, -9px 0;
  }
  .imgwrap img { max-width: 100%; max-height: 100%; object-fit: contain; box-shadow: 0 4px 20px rgba(0,0,0,0.3); border-radius: 4px; }
  .bar { flex: 0 0 auto; height: 26px; display: flex; align-items: center; padding: 0 14px; color: var(--text3);
    font-size: 11.5px; border-top: 1px solid var(--border); }
  .binph { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; color: var(--text3); }
  .binph .bi { font-size: 38px; opacity: 0.5; }
  .binph .t { color: var(--text2); font-size: 13px; font-family: var(--font-mono); }
  .binph .s { font-size: 11.5px; }
</style>
