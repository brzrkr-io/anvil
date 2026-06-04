// CodeMirror 6 theme + syntax highlighting derived from the active Mineral
// palette (themes.ts). Replaces Monaco's defineTheme. One function returns the
// editor-chrome theme plus a Lezer HighlightStyle so code colors match the app.
import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import { Prec, type Extension } from "@codemirror/state";
import { themes } from "$lib/themes";

export function cmTheme(name: string): Extension {
  const ui = themes[name]?.ui ?? themes["solarized-dark"].ui;
  const dark = !name.includes("light") && !name.includes("dawn");

  const theme = EditorView.theme(
    {
      // Transparent so the pane's (alpha-aware) background shows through when
      // window translucency is on; at full opacity the pane bg is solid --bg.
      "&": { color: ui.text, backgroundColor: "transparent", height: "100%" },
      // line-height is owned by Editor's fontTheme (from the user setting).
      ".cm-scroller": { fontFamily: "var(--font-mono)" },
      ".cm-content": { caretColor: ui.accent, padding: "2px 0" },
      ".cm-line": { padding: "0 4px" },
      ".cm-cursor, .cm-dropCursor": { borderLeftColor: ui.accent, borderLeftWidth: "2px" },
      "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": {
        backgroundColor: ui.sel,
      },
      ".cm-panels": { backgroundColor: ui.panel, color: ui.text },
      ".cm-panels.cm-panels-bottom": { borderTop: `1px solid ${ui.border}` },
      ".cm-searchMatch": { backgroundColor: ui.panel2, outline: `1px solid ${ui.border}` },
      ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: ui.sel },
      ".cm-panel.cm-search": { padding: "6px 8px", fontFamily: "var(--font-ui)", fontSize: "12px", display: "flex", flexWrap: "wrap", alignItems: "center", gap: "6px" },
      ".cm-panel.cm-search label": { display: "inline-flex", alignItems: "center", gap: "3px", color: ui.text2, fontSize: "11px" },
      ".cm-panel.cm-search input[type=text], .cm-textfield": {
        backgroundColor: ui.bg,
        color: ui.text,
        border: `1px solid ${ui.border}`,
        borderRadius: "5px",
        padding: "3px 7px",
        fontFamily: "var(--font-mono)",
        fontSize: "12px",
        outline: "none",
      },
      ".cm-panel.cm-search input[type=text]:focus, .cm-textfield:focus": { borderColor: ui.accent },
      ".cm-panel.cm-search button, .cm-button": {
        backgroundColor: ui.panel2,
        color: ui.text,
        border: `1px solid ${ui.border}`,
        borderRadius: "5px",
        padding: "3px 9px",
        fontSize: "11px",
        cursor: "pointer",
        backgroundImage: "none",
      },
      ".cm-panel.cm-search button:hover, .cm-button:hover": { backgroundColor: ui.sel, borderColor: ui.accent },
      ".cm-panel.cm-search button[name=close]": {
        background: "none",
        border: "none",
        color: ui.text3,
        fontSize: "16px",
        padding: "0 4px",
        position: "static",
      },
      ".cm-panel.cm-search button[name=close]:hover": { color: ui.text, backgroundColor: "transparent" },
      ".cm-activeLine": { backgroundColor: `color-mix(in srgb, ${ui.panel2} 55%, transparent)` },
      ".cm-selectionMatch": { backgroundColor: ui.panel2 },
      "&.cm-focused .cm-matchingBracket, .cm-matchingBracket": {
        backgroundColor: `color-mix(in srgb, ${ui.accent} 16%, transparent)`,
        outline: `1px solid color-mix(in srgb, ${ui.accent} 55%, transparent)`,
        borderRadius: "2px",
      },
      ".cm-gutters": { backgroundColor: ui.bg, color: ui.text3, border: "none" },
      ".cm-lineNumbers .cm-gutterElement": { padding: "0 6px 0 14px", minWidth: "32px" },
      ".cm-activeLineGutter": { backgroundColor: "transparent", color: ui.accent },
      ".cm-foldPlaceholder": {
        backgroundColor: ui.panel2,
        color: ui.text3,
        border: "none",
        margin: "0 4px",
        padding: "0 6px",
        borderRadius: "5px",
      },
      ".cm-tooltip": {
        backgroundColor: ui.panel,
        border: `1px solid ${ui.border}`,
        borderRadius: "8px",
        color: ui.text,
        boxShadow: "0 8px 24px rgba(0,0,0,0.35)",
      },
      ".cm-tooltip-autocomplete > ul > li": { padding: "2px 8px" },
      ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: ui.sel, color: ui.text },
      ".cm-tooltip.cm-tooltip-autocomplete > ul > li .cm-completionIcon": { color: ui.text3 },
      ".cm-tooltip.cm-completionInfo": { backgroundColor: ui.panel2, borderColor: ui.border },
      ".cm-lsp-hover": { padding: "7px 10px", whiteSpace: "pre-wrap", maxWidth: "520px", fontFamily: "var(--font-mono)", fontSize: "12px", lineHeight: "1.5" },
      ".cm-sig": { padding: "6px 10px", whiteSpace: "pre-wrap", maxWidth: "520px", fontFamily: "var(--font-mono)", fontSize: "12px", color: ui.text2 },
      ".cm-inlay": { color: ui.text3, backgroundColor: ui.panel2, borderRadius: "4px", padding: "0 4px", margin: "0 1px", fontSize: "85%", fontStyle: "normal", opacity: "0.9" },
      ".cm-ghost": { color: ui.text3, opacity: "0.55", fontStyle: "italic" },
      ".cm-tooltip.cm-tooltip-lint": { backgroundColor: ui.panel, border: `1px solid ${ui.border}`, borderRadius: "8px" },
      ".cm-diagnostic": { padding: "4px 8px", borderLeft: "none" },
      ".cm-diagnostic-error": { borderLeft: `3px solid ${ui.red}` },
      ".cm-diagnostic-warning": { borderLeft: `3px solid ${ui.yellow}` },
      ".cm-diagnostic-info": { borderLeft: `3px solid ${ui.blue}` },
      ".cm-lintRange-error": { backgroundImage: "none", borderBottom: `2px dotted ${ui.red}` },
      ".cm-lintRange-warning": { backgroundImage: "none", borderBottom: `2px dotted ${ui.yellow}` },
    },
    { dark },
  );

  const syn = themes[name]?.syntax ?? {};
  const s = (role: string, fb: string) => syn[role] ?? fb;

  const hl = HighlightStyle.define([
    // Comments recede: blend text3 toward bg so they read clearly as secondary
    // in every theme (raw text3 is too close to body text in e.g. gruvbox-light).
    { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], color: s("comment", `color-mix(in srgb, ${ui.text3} 58%, ${ui.bg})`), fontStyle: "italic" },
    { tag: [t.keyword, t.modifier, t.controlKeyword, t.operatorKeyword, t.definitionKeyword, t.moduleKeyword], color: s("keyword", ui.purple) },
    { tag: [t.string, t.special(t.string), t.docString], color: s("string", ui.green) },
    { tag: [t.number, t.integer, t.float], color: s("number", ui.yellow) },
    { tag: [t.bool, t.null, t.atom], color: s("constant", ui.accent2) },
    { tag: [t.typeName, t.className, t.namespace, t.definition(t.typeName)], color: s("type", ui.teal) },
    { tag: [t.function(t.variableName), t.function(t.propertyName), t.macroName, t.labelName], color: s("function", ui.blue) },
    { tag: [t.propertyName, t.special(t.propertyName)], color: s("property", ui.blue) },
    { tag: [t.tagName, t.angleBracket], color: s("tag", ui.red) },
    { tag: t.attributeName, color: s("attribute", ui.yellow) },
    { tag: t.attributeValue, color: s("string", ui.green) },
    { tag: [t.variableName, t.definition(t.variableName), t.local(t.variableName)], color: s("variable", ui.text) },
    { tag: [t.constant(t.variableName), t.standard(t.variableName)], color: s("constant", ui.accent2) },
    { tag: [t.operator, t.punctuation, t.separator, t.bracket, t.brace, t.paren], color: s("operator", ui.text2) },
    { tag: [t.regexp], color: s("regexp", ui.teal) },
    { tag: [t.escape, t.special(t.brace)], color: s("escape", ui.teal) },
    { tag: [t.heading, t.heading1, t.heading2, t.heading3], color: s("heading", ui.accent), fontWeight: "bold" },
    { tag: [t.link, t.url], color: s("link", ui.blue), textDecoration: "underline" },
    { tag: t.strong, fontWeight: "bold" },
    { tag: t.emphasis, fontStyle: "italic" },
    { tag: [t.meta, t.documentMeta, t.processingInstruction], color: s("meta", ui.text3) },
    { tag: t.invalid, color: ui.red },
  ]);

  // Prec.high so our palette wins over basicSetup's bundled default highlight.
  return [theme, Prec.high(syntaxHighlighting(hl))];
}
