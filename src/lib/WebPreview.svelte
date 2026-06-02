<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  let { url = $bindable("http://localhost:5173") }: { url?: string } = $props();
  let draft = $state(url);
  let frameKey = $state(0);
  function go() {
    let u = draft.trim();
    if (u && !/^https?:\/\//.test(u)) u = "http://" + u;
    url = u;
    draft = u;
    frameKey += 1;
  }
</script>

<div class="wp">
  <div class="wp-bar">
    <button class="wp-btn" title="Reload" onclick={() => (frameKey += 1)}><Icon name="refresh" size={13} /></button>
    <input
      class="wp-url"
      bind:value={draft}
      spellcheck="false"
      onkeydown={(e) => { if (e.key === "Enter") go(); }}
      placeholder="http://localhost:5173"
    />
    <button class="wp-btn" title="Go" onclick={go}><Icon name="play" size={13} /></button>
  </div>
  {#key frameKey}
    <iframe class="wp-frame" src={url} title="Web preview" sandbox="allow-scripts allow-same-origin allow-forms"></iframe>
  {/key}
</div>

<style>
  .wp { display: flex; flex-direction: column; height: 100%; background: var(--bg); }
  .wp-bar { display: flex; align-items: center; gap: 6px; padding: 6px 8px; border-bottom: 1px solid var(--border); background: var(--panel); }
  .wp-btn { display: inline-flex; align-items: center; justify-content: center; width: 26px; height: 24px; border: 1px solid var(--border); border-radius: var(--radius); background: var(--panel2); color: var(--text2); cursor: default; }
  .wp-btn:hover { background: var(--sel); color: var(--text); }
  .wp-url { flex: 1; height: 24px; padding: 0 8px; border: 1px solid var(--border); border-radius: var(--radius); background: var(--bg); color: var(--text); font-family: var(--font-ui); font-size: 12px; }
  .wp-url:focus { outline: none; border-color: var(--accent); }
  .wp-frame { flex: 1; width: 100%; border: 0; background: #fff; }
</style>
