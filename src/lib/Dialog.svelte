<script lang="ts">
  import { activeDialog } from "$lib/dialog";

  let req = $state<import("$lib/dialog").DialogRequest | null>(null);
  let inputVal = $state("");
  let inputEl = $state<HTMLInputElement | undefined>(undefined);

  activeDialog.subscribe((r) => {
    req = r;
    if (r?.kind === "text") inputVal = r.value ?? "";
  });

  $effect(() => {
    if (req && inputEl) inputEl.focus();
  });

  function ok() {
    if (!req) return;
    if (req.kind === "text") {
      const v = inputVal;
      req.resolve(v);
    } else {
      req.resolve(true);
    }
    activeDialog.set(null);
  }

  function cancel() {
    if (!req) return;
    if (req.kind === "text") req.resolve(null);
    else req.resolve(false);
    activeDialog.set(null);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") { e.preventDefault(); ok(); }
    else if (e.key === "Escape") { e.preventDefault(); cancel(); }
  }
</script>

{#if req}
  <div class="dlg-scrim" role="presentation" onclick={cancel} onkeydown={onKeydown}>
    <div class="dlg" role="dialog" aria-modal="true" aria-label={req.title} onclick={(e) => e.stopPropagation()} onkeydown={onKeydown}>
      <div class="dlg-title">{req.title}</div>
      {#if req.message}<div class="dlg-msg">{req.message}</div>{/if}
      {#if req.kind === "text"}
        <input
          class="dlg-input"
          type="text"
          bind:value={inputVal}
          bind:this={inputEl}
          placeholder={req.placeholder ?? ""}
          onkeydown={onKeydown}
          autocomplete="off"
          spellcheck={false}
        />
      {/if}
      <div class="dlg-actions">
        <button class="dlg-btn ghost" onclick={cancel}>Cancel</button>
        <button class="dlg-btn primary {req.kind === 'confirm' && req.danger ? 'danger' : ''}" onclick={ok}>
          {req.okLabel ?? (req.kind === "confirm" ? "OK" : "OK")}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .dlg-scrim {
    position: fixed; inset: 0; background: var(--glass-scrim);
    backdrop-filter: blur(6px); -webkit-backdrop-filter: blur(6px);
    display: flex; align-items: center; justify-content: center; z-index: 300;
  }
  .dlg {
    width: 340px; max-width: 92vw;
    background: var(--glass);
    backdrop-filter: blur(var(--blur)) saturate(1.3);
    -webkit-backdrop-filter: blur(var(--blur)) saturate(1.3);
    border: 1px solid var(--border); border-radius: 8px;
    box-shadow: var(--elev-3), inset 0 1px 0 var(--hairline);
    padding: 18px 18px 14px; display: flex; flex-direction: column; gap: 8px;
  }
  .dlg-title {
    font-size: 13px; font-weight: 600; color: var(--text); line-height: 1.3;
  }
  .dlg-msg {
    font-size: 12px; color: var(--text2); line-height: 1.45;
  }
  .dlg-input {
    width: 100%; box-sizing: border-box;
    background: var(--panel2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); font-family: var(--font-ui); font-size: 12.5px;
    padding: 6px 8px; outline: none;
  }
  .dlg-input:focus { border-color: var(--accent); }
  .dlg-actions {
    display: flex; justify-content: flex-end; gap: 6px; margin-top: 4px;
  }
  .dlg-btn {
    padding: 5px 14px; border-radius: 6px; font-family: var(--font-ui);
    font-size: 12.5px; font-weight: 500; cursor: pointer; border: none;
  }
  .dlg-btn.ghost {
    background: transparent; border: 1px solid var(--border); color: var(--text2);
  }
  .dlg-btn.ghost:hover { background: color-mix(in srgb, var(--text) 6%, transparent); }
  .dlg-btn.primary {
    background: var(--accent); color: var(--bg);
  }
  .dlg-btn.primary:hover { filter: brightness(1.08); }
  .dlg-btn.primary.danger {
    background: var(--red); color: #fff;
  }
  .dlg-btn.primary.danger:hover { filter: brightness(1.1); }
</style>
