<script lang="ts">
  // Read-only, syntax-highlighted viewer for command output (Helm values, k8s
  // describe/manifest, Flux values, …). Reuses the editor's CodeMirror language
  // map + themed highlight style so colors match the editor exactly. Virtualized,
  // so multi-thousand-line "helm get values" output renders without freezing.
  import { onMount, onDestroy } from "svelte";
  import { EditorState, Compartment } from "@codemirror/state";
  import { EditorView, lineNumbers } from "@codemirror/view";
  import { cmTheme } from "$lib/cm-theme";
  import { cmLang } from "$lib/cm-lang";
  import { activeTheme } from "$lib/themes";

  let { text = "", lang = "", wrap = false, numbers = false }:
    { text?: string; lang?: string; wrap?: boolean; numbers?: boolean } = $props();

  let host: HTMLDivElement;
  let view: EditorView | undefined;
  const themeComp = new Compartment();
  const langComp = new Compartment();
  let unsub: (() => void) | undefined;

  // kubectl/helm/flux output is YAML unless it's clearly JSON; callers can force a
  // language via the `lang` prop (e.g. "json", "hcl", "sh").
  function detect(t: string): string {
    if (lang) return lang;
    const s = t.trimStart();
    return s.startsWith("{") || s.startsWith("[") ? "json" : "yaml";
  }
  const langExt = (t: string) => cmLang(`x.${detect(t)}`);

  onMount(() => {
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: text,
        extensions: [
          EditorState.readOnly.of(true),
          EditorView.editable.of(false),
          ...(numbers ? [lineNumbers()] : []),
          ...(wrap ? [EditorView.lineWrapping] : []),
          langComp.of(langExt(text)),
          themeComp.of(cmTheme($activeTheme)),
        ],
      }),
    });
    unsub = activeTheme.subscribe((n) => view?.dispatch({ effects: themeComp.reconfigure(cmTheme(n)) }));
  });
  onDestroy(() => { unsub?.(); view?.destroy(); });

  // Re-fill the document (and re-pick the language) whenever the text changes.
  $effect(() => {
    const t = text;
    if (!view) return;
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: t },
      effects: langComp.reconfigure(langExt(t)),
    });
  });
</script>

<div class="codeview" bind:this={host}></div>

<style>
  .codeview { height: 100%; min-height: 0; overflow: hidden; }
  .codeview :global(.cm-editor) { height: 100%; }
  .codeview :global(.cm-scroller) { overflow: auto; }
</style>
