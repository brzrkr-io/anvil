// Inline color swatches (#5): a small clickable square before every hex/rgb(a)
// color literal. Clicking opens a native color picker that rewrites the value.
import { EditorView, Decoration, WidgetType, ViewPlugin, MatchDecorator, type DecorationSet, type ViewUpdate } from "@codemirror/view";
import type { Extension } from "@codemirror/state";

const COLOR_RE = /#(?:[0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})\b|rgba?\([^)]*\)/g;

function toHex(c: string): string {
  if (c.startsWith("#")) {
    if (c.length === 4) return "#" + [...c.slice(1)].map((x) => x + x).join("");
    if (c.length === 5) return "#" + [...c.slice(1, 4)].map((x) => x + x).join(""); // #rgba → drop alpha
    if (c.length === 9) return c.slice(0, 7); // #rrggbbaa → drop alpha for the picker
    return c.slice(0, 7);
  }
  const m = c.match(/rgba?\(\s*(\d+)[,\s]+(\d+)[,\s]+(\d+)/);
  if (!m) return "#000000";
  const h = (n: number) => Math.max(0, Math.min(255, n)).toString(16).padStart(2, "0");
  return "#" + h(+m[1]) + h(+m[2]) + h(+m[3]);
}

class SwatchWidget extends WidgetType {
  color: string;
  from: number;
  to: number;
  constructor(color: string, from: number, to: number) { super(); this.color = color; this.from = from; this.to = to; }
  eq(o: SwatchWidget) { return o.color === this.color && o.from === this.from; }
  toDOM(view: EditorView) {
    const wrap = document.createElement("span");
    wrap.className = "cm-color-swatch";
    wrap.style.backgroundColor = this.color;
    const input = document.createElement("input");
    input.type = "color";
    input.value = toHex(this.color);
    input.className = "cm-color-input";
    input.onchange = () => {
      view.dispatch({ changes: { from: this.from, to: this.to, insert: input.value } });
    };
    wrap.appendChild(input);
    return wrap;
  }
  ignoreEvent() { return false; }
}

const matcher = new MatchDecorator({
  regexp: COLOR_RE,
  decorate(add, from, to, match) {
    add(from, from, Decoration.widget({ widget: new SwatchWidget(match[0], from, to), side: -1 }));
  },
});

export function colorSwatches(): Extension {
  return ViewPlugin.fromClass(
    class {
      swatches: DecorationSet;
      constructor(view: EditorView) { this.swatches = matcher.createDeco(view); }
      update(u: ViewUpdate) { this.swatches = matcher.updateDeco(u, this.swatches); }
    },
    { decorations: (v) => v.swatches },
  );
}
