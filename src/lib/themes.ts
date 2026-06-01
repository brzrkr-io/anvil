import { writable } from "svelte/store";

export type Theme = {
  ui: Record<string, string>;
  xterm: Record<string, string>;
  syntax?: Record<string, string>;
};

export const themes: Record<string, Theme> = {
  // Anvil's signature pair — neutral canvas, warm coral/ember brand accents,
  // full-hue syntax separation tuned to pop and read well for long coding sessions.
  "anvil-dark": {
    ui: {
      bg: "#111113", panel: "#18181b", panel2: "#222226", border: "#2e2e33",
      text: "#ededf0", text2: "#b8b8be", text3: "#7d7d85", sel: "#2d2630",
      accent: "#ef8a5d", accent2: "#e3893f", green: "#94c47d", red: "#e5604d",
      blue: "#5fb6c9", purple: "#c98bdb", teal: "#4fc1b0", yellow: "#e3a857",
    },
    xterm: {
      background: "#111113", foreground: "#ededf0", cursor: "#ef8a5d",
      cursorAccent: "#111113", selectionBackground: "#2d2630",
      black: "#18181b", red: "#e5604d", green: "#94c47d", yellow: "#e3a857",
      blue: "#5fb6c9", magenta: "#c98bdb", cyan: "#4fc1b0", white: "#b8b8be",
      brightBlack: "#7d7d85", brightRed: "#ef7060", brightGreen: "#a6d18f",
      brightYellow: "#f0b76a", brightBlue: "#72c5d8", brightMagenta: "#d6a0e8",
      brightCyan: "#62d0bf", brightWhite: "#ededf0",
    },
    syntax: {
      comment: "#6b6b73", keyword: "#c98bdb", string: "#94c47d", number: "#e3a857",
      type: "#4fc1b0", function: "#ef8a5d", property: "#5fb6c9", variable: "#ededf0",
      constant: "#e3a857", operator: "#8a8a92", tag: "#e5604d", attribute: "#e3a857",
      regexp: "#4fc1b0", escape: "#ef8a5d", heading: "#ef8a5d", link: "#5fb6c9", meta: "#7d7d85",
    },
  },
  "anvil-light": {
    ui: {
      bg: "#fafafa", panel: "#f2f2f3", panel2: "#e9e9eb", border: "#dcdce0",
      text: "#1c1c1f", text2: "#4a4a50", text3: "#8a8a90", sel: "#f0e2dc",
      accent: "#c85a3c", accent2: "#c5611c", green: "#4e8a3a", red: "#c0392b",
      blue: "#2d7a99", purple: "#9a4ab0", teal: "#1d8a7a", yellow: "#b07219",
    },
    xterm: {
      background: "#fafafa", foreground: "#1c1c1f", cursor: "#c85a3c",
      cursorAccent: "#fafafa", selectionBackground: "#f0e2dc",
      black: "#1c1c1f", red: "#c0392b", green: "#4e8a3a", yellow: "#b07219",
      blue: "#2d7a99", magenta: "#9a4ab0", cyan: "#1d8a7a", white: "#4a4a50",
      brightBlack: "#8a8a90", brightRed: "#d0473a", brightGreen: "#5e9a48",
      brightYellow: "#c4892f", brightBlue: "#3d8aa9", brightMagenta: "#aa5ac0",
      brightCyan: "#2d9a8a", brightWhite: "#1c1c1f",
    },
    syntax: {
      comment: "#a0a0a6", keyword: "#9a4ab0", string: "#4e8a3a", number: "#b07219",
      type: "#1d8a7a", function: "#c25a32", property: "#2d7a99", variable: "#1c1c1f",
      constant: "#b07219", operator: "#6e6e74", tag: "#c0392b", attribute: "#b07219",
      regexp: "#1d8a7a", escape: "#c25a32", heading: "#c25a32", link: "#2d7a99", meta: "#a0a0a6",
    },
  },
  "solarized-dark": {
    ui: {
      bg: "#002b36", panel: "#073642", panel2: "#0a4150", border: "#0f4a5a",
      text: "#93a1a1", text2: "#839496", text3: "#586e75", sel: "#134556",
      accent: "#268bd2", accent2: "#cb4b16", green: "#859900", red: "#dc322f",
      blue: "#268bd2", purple: "#6c71c4", teal: "#2aa198", yellow: "#b58900",
    },
    xterm: {
      background: "#002b36", foreground: "#93a1a1", cursor: "#268bd2",
      cursorAccent: "#002b36", selectionBackground: "#134556",
      black: "#073642", red: "#dc322f", green: "#859900", yellow: "#b58900",
      blue: "#268bd2", magenta: "#d33682", cyan: "#2aa198", white: "#eee8d5",
      brightBlack: "#586e75", brightRed: "#cb4b16", brightGreen: "#859900",
      brightYellow: "#657b83", brightBlue: "#839496", brightMagenta: "#6c71c4",
      brightCyan: "#93a1a1", brightWhite: "#fdf6e3",
    },
    syntax: {
      comment: "#586e75", keyword: "#859900", string: "#2aa198", number: "#d33682",
      type: "#b58900", function: "#268bd2", property: "#268bd2", variable: "#93a1a1",
      constant: "#d33682", operator: "#859900", tag: "#268bd2", attribute: "#b58900",
      regexp: "#dc322f", escape: "#cb4b16", heading: "#268bd2", link: "#268bd2", meta: "#586e75",
    },
  },
  "solarized-light": {
    ui: {
      bg: "#fdf6e3", panel: "#eee8d5", panel2: "#e7e0cb", border: "#d9d2b8",
      text: "#586e75", text2: "#657b83", text3: "#93a1a1", sel: "#eee8d5",
      accent: "#268bd2", accent2: "#cb4b16", green: "#859900", red: "#dc322f",
      blue: "#268bd2", purple: "#6c71c4", teal: "#2aa198", yellow: "#b58900",
    },
    xterm: {
      background: "#fdf6e3", foreground: "#586e75", cursor: "#268bd2",
      cursorAccent: "#fdf6e3", selectionBackground: "#eee8d5",
      black: "#073642", red: "#dc322f", green: "#859900", yellow: "#b58900",
      blue: "#268bd2", magenta: "#d33682", cyan: "#2aa198", white: "#eee8d5",
      brightBlack: "#586e75", brightRed: "#cb4b16", brightGreen: "#859900",
      brightYellow: "#657b83", brightBlue: "#839496", brightMagenta: "#6c71c4",
      brightCyan: "#93a1a1", brightWhite: "#fdf6e3",
    },
    syntax: {
      comment: "#93a1a1", keyword: "#859900", string: "#2aa198", number: "#d33682",
      type: "#b58900", function: "#268bd2", property: "#268bd2", variable: "#586e75",
      constant: "#d33682", operator: "#859900", tag: "#268bd2", attribute: "#b58900",
      regexp: "#dc322f", escape: "#cb4b16", heading: "#268bd2", link: "#268bd2", meta: "#93a1a1",
    },
  },
  "amber-dark": {
    ui: {
      bg: "#1a1518", panel: "#221b1f", panel2: "#2a2127", border: "#372a30",
      text: "#f0e6e2", text2: "#b39a93", text3: "#7c655f", sel: "#36242a",
      accent: "#f5874f", accent2: "#f0699a", green: "#9bc777", red: "#ef6f7a",
      blue: "#7ab0c4", purple: "#cf9ce0", teal: "#3fb59a", yellow: "#f4bd5a",
    },
    xterm: {
      background: "#1a1518", foreground: "#f0e6e2", cursor: "#f5874f",
      cursorAccent: "#1a1518", selectionBackground: "#36242a",
      black: "#221b1f", red: "#ef6f7a", green: "#9bc777", yellow: "#f4bd5a",
      blue: "#7ab0c4", magenta: "#cf9ce0", cyan: "#3fb59a", white: "#f0e6e2",
      brightBlack: "#5a4850", brightRed: "#f0699a", brightGreen: "#acd488",
      brightYellow: "#ffce6a", brightBlue: "#8cc0d4", brightMagenta: "#dbacf0",
      brightCyan: "#6fe6ce", brightWhite: "#fdf4f0",
    },
    syntax: {
      comment: "#7c655f", keyword: "#f0699a", string: "#9bc777", number: "#f4bd5a",
      type: "#3fb59a", function: "#7ab0c4", property: "#7ab0c4", variable: "#f0e6e2",
      constant: "#cf9ce0", operator: "#f5874f", tag: "#ef6f7a", attribute: "#f4bd5a",
      regexp: "#9bc777", escape: "#f5874f", heading: "#f5874f", link: "#7ab0c4", meta: "#b39a93",
    },
  },
  "tokyo-night": {
    ui: {
      bg: "#1a1b26", panel: "#16161e", panel2: "#1f2335", border: "#292e42",
      text: "#c0caf5", text2: "#a9b1d6", text3: "#565f89", sel: "#283457",
      accent: "#7aa2f7", accent2: "#bb9af7", green: "#9ece6a", red: "#f7768e",
      blue: "#7aa2f7", purple: "#bb9af7", teal: "#7dcfff", yellow: "#e0af68",
    },
    xterm: {
      background: "#1a1b26", foreground: "#c0caf5", cursor: "#c0caf5",
      cursorAccent: "#1a1b26", selectionBackground: "#283457",
      black: "#15161e", red: "#f7768e", green: "#9ece6a", yellow: "#e0af68",
      blue: "#7aa2f7", magenta: "#bb9af7", cyan: "#7dcfff", white: "#a9b1d6",
      brightBlack: "#414868", brightRed: "#f7768e", brightGreen: "#9ece6a",
      brightYellow: "#e0af68", brightBlue: "#7aa2f7", brightMagenta: "#bb9af7",
      brightCyan: "#7dcfff", brightWhite: "#c0caf5",
    },
    syntax: {
      comment: "#565f89", keyword: "#bb9af7", string: "#9ece6a", number: "#ff9e64",
      type: "#2ac3de", function: "#7aa2f7", property: "#7dcfff", variable: "#c0caf5",
      constant: "#ff9e64", operator: "#89ddff", tag: "#f7768e", attribute: "#bb9af7",
      regexp: "#b4f9f8", escape: "#89ddff", heading: "#7aa2f7", link: "#7dcfff", meta: "#565f89",
    },
  },
  "gruvbox-dark": {
    ui: {
      bg: "#282828", panel: "#3c3836", panel2: "#504945", border: "#665c54",
      text: "#ebdbb2", text2: "#d5c4a1", text3: "#928374", sel: "#504945",
      accent: "#fabd2f", accent2: "#fe8019", green: "#b8bb26", red: "#fb4934",
      blue: "#83a598", purple: "#d3869b", teal: "#8ec07c", yellow: "#fabd2f",
    },
    xterm: {
      background: "#282828", foreground: "#ebdbb2", cursor: "#ebdbb2",
      cursorAccent: "#282828", selectionBackground: "#504945",
      black: "#282828", red: "#cc241d", green: "#98971a", yellow: "#d79921",
      blue: "#458588", magenta: "#b16286", cyan: "#689d6a", white: "#a89984",
      brightBlack: "#928374", brightRed: "#fb4934", brightGreen: "#b8bb26",
      brightYellow: "#fabd2f", brightBlue: "#83a598", brightMagenta: "#d3869b",
      brightCyan: "#8ec07c", brightWhite: "#ebdbb2",
    },
    syntax: {
      comment: "#928374", keyword: "#fb4934", string: "#b8bb26", number: "#d3869b",
      type: "#fabd2f", function: "#b8bb26", property: "#83a598", variable: "#ebdbb2",
      constant: "#d3869b", operator: "#fe8019", tag: "#8ec07c", attribute: "#fabd2f",
      regexp: "#b8bb26", escape: "#fe8019", heading: "#b8bb26", link: "#83a598", meta: "#fe8019",
    },
  },
  "catppuccin-mocha": {
    ui: {
      bg: "#1e1e2e", panel: "#181825", panel2: "#313244", border: "#45475a",
      text: "#cdd6f4", text2: "#bac2de", text3: "#6c7086", sel: "#45475a",
      accent: "#cba6f7", accent2: "#f38ba8", green: "#a6e3a1", red: "#f38ba8",
      blue: "#89b4fa", purple: "#cba6f7", teal: "#94e2d5", yellow: "#f9e2af",
    },
    xterm: {
      background: "#1e1e2e", foreground: "#cdd6f4", cursor: "#f5e0dc",
      cursorAccent: "#1e1e2e", selectionBackground: "#45475a",
      black: "#45475a", red: "#f38ba8", green: "#a6e3a1", yellow: "#f9e2af",
      blue: "#89b4fa", magenta: "#f5c2e7", cyan: "#94e2d5", white: "#bac2de",
      brightBlack: "#585b70", brightRed: "#f38ba8", brightGreen: "#a6e3a1",
      brightYellow: "#f9e2af", brightBlue: "#89b4fa", brightMagenta: "#f5c2e7",
      brightCyan: "#94e2d5", brightWhite: "#a6adc8",
    },
    syntax: {
      comment: "#6c7086", keyword: "#cba6f7", string: "#a6e3a1", number: "#fab387",
      type: "#f9e2af", function: "#89b4fa", property: "#89dceb", variable: "#cdd6f4",
      constant: "#fab387", operator: "#89dceb", tag: "#f38ba8", attribute: "#f9e2af",
      regexp: "#f5c2e7", escape: "#f5c2e7", heading: "#89b4fa", link: "#89dceb", meta: "#6c7086",
    },
  },
  "nord": {
    ui: {
      bg: "#2e3440", panel: "#3b4252", panel2: "#434c5e", border: "#4c566a",
      text: "#eceff4", text2: "#e5e9f0", text3: "#d8dee9", sel: "#434c5e",
      accent: "#88c0d0", accent2: "#81a1c1", green: "#a3be8c", red: "#bf616a",
      blue: "#81a1c1", purple: "#b48ead", teal: "#88c0d0", yellow: "#ebcb8b",
    },
    xterm: {
      background: "#2e3440", foreground: "#d8dee9", cursor: "#d8dee9",
      cursorAccent: "#2e3440", selectionBackground: "#434c5e",
      black: "#3b4252", red: "#bf616a", green: "#a3be8c", yellow: "#ebcb8b",
      blue: "#81a1c1", magenta: "#b48ead", cyan: "#88c0d0", white: "#e5e9f0",
      brightBlack: "#4c566a", brightRed: "#bf616a", brightGreen: "#a3be8c",
      brightYellow: "#ebcb8b", brightBlue: "#81a1c1", brightMagenta: "#b48ead",
      brightCyan: "#8fbcbb", brightWhite: "#eceff4",
    },
    syntax: {
      comment: "#616e88", keyword: "#81a1c1", string: "#a3be8c", number: "#b48ead",
      type: "#8fbcbb", function: "#88c0d0", property: "#d8dee9", variable: "#d8dee9",
      constant: "#b48ead", operator: "#81a1c1", tag: "#81a1c1", attribute: "#8fbcbb",
      regexp: "#ebcb8b", escape: "#ebcb8b", heading: "#88c0d0", link: "#88c0d0", meta: "#5e81ac",
    },
  },
  "dracula": {
    ui: {
      bg: "#282a36", panel: "#1e1f29", panel2: "#343746", border: "#44475a",
      text: "#f8f8f2", text2: "#e2e2dc", text3: "#6272a4", sel: "#44475a",
      accent: "#bd93f9", accent2: "#ff79c6", green: "#50fa7b", red: "#ff5555",
      blue: "#6272a4", purple: "#bd93f9", teal: "#8be9fd", yellow: "#f1fa8c",
    },
    xterm: {
      background: "#282a36", foreground: "#f8f8f2", cursor: "#f8f8f2",
      cursorAccent: "#282a36", selectionBackground: "#44475a",
      black: "#21222c", red: "#ff5555", green: "#50fa7b", yellow: "#f1fa8c",
      blue: "#bd93f9", magenta: "#ff79c6", cyan: "#8be9fd", white: "#f8f8f2",
      brightBlack: "#6272a4", brightRed: "#ff6e6e", brightGreen: "#69ff94",
      brightYellow: "#ffffa5", brightBlue: "#d6acff", brightMagenta: "#ff92df",
      brightCyan: "#a4ffff", brightWhite: "#ffffff",
    },
    syntax: {
      comment: "#6272a4", keyword: "#ff79c6", string: "#f1fa8c", number: "#bd93f9",
      type: "#8be9fd", function: "#50fa7b", property: "#f8f8f2", variable: "#f8f8f2",
      constant: "#bd93f9", operator: "#ff79c6", tag: "#ff79c6", attribute: "#50fa7b",
      regexp: "#f1fa8c", escape: "#ff79c6", heading: "#bd93f9", link: "#8be9fd", meta: "#6272a4",
    },
  },
  "rose-pine": {
    ui: {
      bg: "#191724", panel: "#1f1d2e", panel2: "#26233a", border: "#403d52",
      text: "#e0def4", text2: "#908caa", text3: "#6e6a86", sel: "#312f44",
      accent: "#c4a7e7", accent2: "#ebbcba", green: "#9ccfd8", red: "#eb6f92",
      blue: "#31748f", purple: "#c4a7e7", teal: "#9ccfd8", yellow: "#f6c177",
    },
    xterm: {
      background: "#191724", foreground: "#e0def4", cursor: "#e0def4",
      cursorAccent: "#191724", selectionBackground: "#312f44",
      black: "#26233a", red: "#eb6f92", green: "#9ccfd8", yellow: "#f6c177",
      blue: "#31748f", magenta: "#c4a7e7", cyan: "#ebbcba", white: "#e0def4",
      brightBlack: "#6e6a86", brightRed: "#eb6f92", brightGreen: "#9ccfd8",
      brightYellow: "#f6c177", brightBlue: "#31748f", brightMagenta: "#c4a7e7",
      brightCyan: "#ebbcba", brightWhite: "#e0def4",
    },
    syntax: {
      comment: "#6e6a86", keyword: "#31748f", string: "#f6c177", number: "#ebbcba",
      type: "#9ccfd8", function: "#ebbcba", property: "#9ccfd8", variable: "#e0def4",
      constant: "#ebbcba", operator: "#908caa", tag: "#31748f", attribute: "#9ccfd8",
      regexp: "#ebbcba", escape: "#c4a7e7", heading: "#c4a7e7", link: "#9ccfd8", meta: "#908caa",
    },
  },
  "rose-pine-dawn": {
    ui: {
      bg: "#faf4ed", panel: "#fffaf3", panel2: "#f2e9e1", border: "#dfdad9",
      text: "#575279", text2: "#797593", text3: "#9893a5", sel: "#eee4da",
      accent: "#907aa9", accent2: "#d7827e", green: "#56949f", red: "#b4637a",
      blue: "#286983", purple: "#907aa9", teal: "#56949f", yellow: "#ea9d34",
    },
    xterm: {
      background: "#faf4ed", foreground: "#575279", cursor: "#575279",
      cursorAccent: "#faf4ed", selectionBackground: "#eee4da",
      black: "#f2e9e1", red: "#b4637a", green: "#56949f", yellow: "#ea9d34",
      blue: "#286983", magenta: "#907aa9", cyan: "#d7827e", white: "#575279",
      brightBlack: "#9893a5", brightRed: "#b4637a", brightGreen: "#56949f",
      brightYellow: "#ea9d34", brightBlue: "#286983", brightMagenta: "#907aa9",
      brightCyan: "#d7827e", brightWhite: "#575279",
    },
    syntax: {
      comment: "#9893a5", keyword: "#286983", string: "#ea9d34", number: "#d7827e",
      type: "#56949f", function: "#d7827e", property: "#56949f", variable: "#575279",
      constant: "#d7827e", operator: "#797593", tag: "#286983", attribute: "#56949f",
      regexp: "#d7827e", escape: "#907aa9", heading: "#907aa9", link: "#56949f", meta: "#797593",
    },
  },
  "gruvbox-light": {
    ui: {
      bg: "#fbf1c7", panel: "#f2e5bc", panel2: "#ebdbb2", border: "#d5c4a1",
      text: "#3c3836", text2: "#504945", text3: "#7c6f64", sel: "#ebdbb2",
      accent: "#b57614", accent2: "#af3a03", green: "#79740e", red: "#9d0006",
      blue: "#076678", purple: "#8f3f71", teal: "#427b58", yellow: "#b57614",
    },
    xterm: {
      background: "#fbf1c7", foreground: "#3c3836", cursor: "#3c3836",
      cursorAccent: "#fbf1c7", selectionBackground: "#ebdbb2",
      black: "#fbf1c7", red: "#9d0006", green: "#79740e", yellow: "#b57614",
      blue: "#076678", magenta: "#8f3f71", cyan: "#427b58", white: "#7c6f64",
      brightBlack: "#928374", brightRed: "#9d0006", brightGreen: "#79740e",
      brightYellow: "#b57614", brightBlue: "#076678", brightMagenta: "#8f3f71",
      brightCyan: "#427b58", brightWhite: "#3c3836",
    },
    syntax: {
      comment: "#928374", keyword: "#9d0006", string: "#79740e", number: "#8f3f71",
      type: "#b57614", function: "#79740e", property: "#076678", variable: "#3c3836",
      constant: "#8f3f71", operator: "#af3a03", tag: "#427b58", attribute: "#b57614",
      regexp: "#79740e", escape: "#af3a03", heading: "#79740e", link: "#076678", meta: "#af3a03",
    },
  },
  "one-dark": {
    ui: {
      bg: "#282c34", panel: "#21252b", panel2: "#2c313a", border: "#3b4048",
      text: "#abb2bf", text2: "#9da5b4", text3: "#5c6370", sel: "#3e4451",
      accent: "#61afef", accent2: "#c678dd", green: "#98c379", red: "#e06c75",
      blue: "#61afef", purple: "#c678dd", teal: "#56b6c2", yellow: "#e5c07b",
    },
    xterm: {
      background: "#282c34", foreground: "#abb2bf", cursor: "#528bff",
      cursorAccent: "#282c34", selectionBackground: "#3e4451",
      black: "#2c313a", red: "#e06c75", green: "#98c379", yellow: "#e5c07b",
      blue: "#61afef", magenta: "#c678dd", cyan: "#56b6c2", white: "#abb2bf",
      brightBlack: "#5c6370", brightRed: "#e06c75", brightGreen: "#98c379",
      brightYellow: "#d19a66", brightBlue: "#61afef", brightMagenta: "#c678dd",
      brightCyan: "#56b6c2", brightWhite: "#ffffff",
    },
    syntax: {
      comment: "#5c6370", keyword: "#c678dd", string: "#98c379", number: "#d19a66",
      type: "#e5c07b", function: "#61afef", property: "#e06c75", variable: "#abb2bf",
      constant: "#d19a66", operator: "#56b6c2", tag: "#e06c75", attribute: "#d19a66",
      regexp: "#98c379", escape: "#56b6c2", heading: "#e06c75", link: "#61afef", meta: "#5c6370",
    },
  },
  "github-dark": {
    ui: {
      bg: "#0d1117", panel: "#161b22", panel2: "#21262d", border: "#30363d",
      text: "#c9d1d9", text2: "#b1bac4", text3: "#8b949e", sel: "#163356",
      accent: "#58a6ff", accent2: "#1f6feb", green: "#3fb950", red: "#ff7b72",
      blue: "#58a6ff", purple: "#bc8cff", teal: "#39c5cf", yellow: "#d29922",
    },
    xterm: {
      background: "#0d1117", foreground: "#c9d1d9", cursor: "#58a6ff",
      cursorAccent: "#0d1117", selectionBackground: "#163356",
      black: "#484f58", red: "#ff7b72", green: "#3fb950", yellow: "#d29922",
      blue: "#58a6ff", magenta: "#bc8cff", cyan: "#39c5cf", white: "#b1bac4",
      brightBlack: "#6e7681", brightRed: "#ffa198", brightGreen: "#56d364",
      brightYellow: "#e3b341", brightBlue: "#79c0ff", brightMagenta: "#d2a8ff",
      brightCyan: "#56d4dd", brightWhite: "#ffffff",
    },
    syntax: {
      comment: "#8b949e", keyword: "#ff7b72", string: "#a5d6ff", number: "#79c0ff",
      type: "#ffa657", function: "#d2a8ff", property: "#79c0ff", variable: "#c9d1d9",
      constant: "#79c0ff", operator: "#ff7b72", tag: "#7ee787", attribute: "#79c0ff",
      regexp: "#a5d6ff", escape: "#79c0ff", heading: "#1f6feb", link: "#a5d6ff", meta: "#8b949e",
    },
  },
  "github-light": {
    ui: {
      bg: "#ffffff", panel: "#f6f8fa", panel2: "#eaeef2", border: "#d0d7de",
      text: "#1f2328", text2: "#424a53", text3: "#6e7781", sel: "#ddf4ff",
      accent: "#0969da", accent2: "#cf222e", green: "#1a7f37", red: "#cf222e",
      blue: "#0969da", purple: "#8250df", teal: "#1b7c83", yellow: "#9a6700",
    },
    xterm: {
      background: "#ffffff", foreground: "#1f2328", cursor: "#0969da",
      cursorAccent: "#ffffff", selectionBackground: "#ddf4ff",
      black: "#24292f", red: "#cf222e", green: "#116329", yellow: "#4d2d00",
      blue: "#0969da", magenta: "#8250df", cyan: "#1b7c83", white: "#6e7781",
      brightBlack: "#57606a", brightRed: "#a40e26", brightGreen: "#1a7f37",
      brightYellow: "#633c01", brightBlue: "#218bff", brightMagenta: "#a475f9",
      brightCyan: "#3192aa", brightWhite: "#8c959f",
    },
    syntax: {
      comment: "#6e7781", keyword: "#cf222e", string: "#0a3069", number: "#0550ae",
      type: "#953800", function: "#8250df", property: "#0550ae", variable: "#1f2328",
      constant: "#0550ae", operator: "#cf222e", tag: "#116329", attribute: "#0550ae",
      regexp: "#0a3069", escape: "#0550ae", heading: "#0969da", link: "#0a3069", meta: "#6e7781",
    },
  },
  "catppuccin-macchiato": {
    ui: {
      bg: "#24273a", panel: "#1e2030", panel2: "#363a4f", border: "#494d64",
      text: "#cad3f5", text2: "#b8c0e0", text3: "#6e738d", sel: "#363a4f",
      accent: "#8aadf4", accent2: "#c6a0f6", green: "#a6da95", red: "#ed8796",
      blue: "#8aadf4", purple: "#c6a0f6", teal: "#8bd5ca", yellow: "#eed49f",
    },
    xterm: {
      background: "#24273a", foreground: "#cad3f5", cursor: "#f4dbd6",
      cursorAccent: "#24273a", selectionBackground: "#363a4f",
      black: "#494d64", red: "#ed8796", green: "#a6da95", yellow: "#eed49f",
      blue: "#8aadf4", magenta: "#f5bde6", cyan: "#8bd5ca", white: "#b8c0e0",
      brightBlack: "#5b6078", brightRed: "#ed8796", brightGreen: "#a6da95",
      brightYellow: "#eed49f", brightBlue: "#8aadf4", brightMagenta: "#f5bde6",
      brightCyan: "#8bd5ca", brightWhite: "#a5adcb",
    },
    syntax: {
      comment: "#6e738d", keyword: "#c6a0f6", string: "#a6da95", number: "#f5a97f",
      type: "#eed49f", function: "#8aadf4", property: "#8bd5ca", variable: "#cad3f5",
      constant: "#f5a97f", operator: "#91d7e3", tag: "#ed8796", attribute: "#eed49f",
      regexp: "#f5bde6", escape: "#f5bde6", heading: "#8aadf4", link: "#8bd5ca", meta: "#6e738d",
    },
  },
  "catppuccin-latte": {
    ui: {
      bg: "#eff1f5", panel: "#e6e9ef", panel2: "#dce0e8", border: "#ccd0da",
      text: "#4c4f69", text2: "#5c5f77", text3: "#8c8fa1", sel: "#dce0e8",
      accent: "#1e66f5", accent2: "#8839ef", green: "#40a02b", red: "#d20f39",
      blue: "#1e66f5", purple: "#8839ef", teal: "#179299", yellow: "#df8e1d",
    },
    xterm: {
      background: "#eff1f5", foreground: "#4c4f69", cursor: "#dc8a78",
      cursorAccent: "#eff1f5", selectionBackground: "#dce0e8",
      black: "#5c5f77", red: "#d20f39", green: "#40a02b", yellow: "#df8e1d",
      blue: "#1e66f5", magenta: "#ea76cb", cyan: "#179299", white: "#acb0be",
      brightBlack: "#6c6f85", brightRed: "#d20f39", brightGreen: "#40a02b",
      brightYellow: "#df8e1d", brightBlue: "#1e66f5", brightMagenta: "#ea76cb",
      brightCyan: "#179299", brightWhite: "#bcc0cc",
    },
    syntax: {
      comment: "#8c8fa1", keyword: "#8839ef", string: "#40a02b", number: "#fe640b",
      type: "#df8e1d", function: "#1e66f5", property: "#179299", variable: "#4c4f69",
      constant: "#fe640b", operator: "#04a5e5", tag: "#d20f39", attribute: "#df8e1d",
      regexp: "#ea76cb", escape: "#ea76cb", heading: "#1e66f5", link: "#179299", meta: "#8c8fa1",
    },
  },
  "ayu-mirage": {
    ui: {
      bg: "#1f2430", panel: "#1a1f29", panel2: "#242936", border: "#2d3441",
      text: "#cccac2", text2: "#b8b5ab", text3: "#707a8c", sel: "#33415e",
      accent: "#ffcc66", accent2: "#73d0ff", green: "#87d96c", red: "#f28779",
      blue: "#73d0ff", purple: "#dfbfff", teal: "#95e6cb", yellow: "#ffd580",
    },
    xterm: {
      background: "#1f2430", foreground: "#cccac2", cursor: "#ffcc66",
      cursorAccent: "#1f2430", selectionBackground: "#33415e",
      black: "#1a1f29", red: "#f28779", green: "#87d96c", yellow: "#ffd173",
      blue: "#73d0ff", magenta: "#dfbfff", cyan: "#95e6cb", white: "#cccac2",
      brightBlack: "#686868", brightRed: "#f28779", brightGreen: "#87d96c",
      brightYellow: "#ffd173", brightBlue: "#73d0ff", brightMagenta: "#dfbfff",
      brightCyan: "#95e6cb", brightWhite: "#ffffff",
    },
    syntax: {
      comment: "#5c6773", keyword: "#ffa759", string: "#d5ff80", number: "#ffcc66",
      type: "#73d0ff", function: "#ffd173", property: "#cccac2", variable: "#cccac2",
      constant: "#d4bfff", operator: "#f29e74", tag: "#5ccfe6", attribute: "#ffd173",
      regexp: "#95e6cb", escape: "#95e6cb", heading: "#ffcc66", link: "#5ccfe6", meta: "#5c6773",
    },
  },
  "monokai": {
    ui: {
      bg: "#272822", panel: "#2d2e27", panel2: "#3e3d32", border: "#49483e",
      text: "#f8f8f2", text2: "#cfcfc2", text3: "#75715e", sel: "#49483e",
      accent: "#66d9ef", accent2: "#f92672", green: "#a6e22e", red: "#f92672",
      blue: "#66d9ef", purple: "#ae81ff", teal: "#a1efe4", yellow: "#e6db74",
    },
    xterm: {
      background: "#272822", foreground: "#f8f8f2", cursor: "#f8f8f0",
      cursorAccent: "#272822", selectionBackground: "#49483e",
      black: "#272822", red: "#f92672", green: "#a6e22e", yellow: "#f4bf75",
      blue: "#66d9ef", magenta: "#ae81ff", cyan: "#a1efe4", white: "#f8f8f2",
      brightBlack: "#75715e", brightRed: "#f92672", brightGreen: "#a6e22e",
      brightYellow: "#f4bf75", brightBlue: "#66d9ef", brightMagenta: "#ae81ff",
      brightCyan: "#a1efe4", brightWhite: "#f9f8f5",
    },
    syntax: {
      comment: "#75715e", keyword: "#f92672", string: "#e6db74", number: "#ae81ff",
      type: "#66d9ef", function: "#a6e22e", property: "#f8f8f2", variable: "#f8f8f2",
      constant: "#ae81ff", operator: "#f92672", tag: "#f92672", attribute: "#a6e22e",
      regexp: "#e6db74", escape: "#ae81ff", heading: "#a6e22e", link: "#66d9ef", meta: "#75715e",
    },
  },
  "everforest": {
    ui: {
      bg: "#2d353b", panel: "#272e33", panel2: "#343f44", border: "#475258",
      text: "#d3c6aa", text2: "#9da9a0", text3: "#859289", sel: "#475258",
      accent: "#a7c080", accent2: "#e67e80", green: "#a7c080", red: "#e67e80",
      blue: "#7fbbb3", purple: "#d699b6", teal: "#83c092", yellow: "#dbbc7f",
    },
    xterm: {
      background: "#2d353b", foreground: "#d3c6aa", cursor: "#d3c6aa",
      cursorAccent: "#2d353b", selectionBackground: "#475258",
      black: "#343f44", red: "#e67e80", green: "#a7c080", yellow: "#dbbc7f",
      blue: "#7fbbb3", magenta: "#d699b6", cyan: "#83c092", white: "#d3c6aa",
      brightBlack: "#868d80", brightRed: "#e67e80", brightGreen: "#a7c080",
      brightYellow: "#dbbc7f", brightBlue: "#7fbbb3", brightMagenta: "#d699b6",
      brightCyan: "#83c092", brightWhite: "#d3c6aa",
    },
    syntax: {
      comment: "#859289", keyword: "#e67e80", string: "#a7c080", number: "#d699b6",
      type: "#dbbc7f", function: "#a7c080", property: "#83c092", variable: "#d3c6aa",
      constant: "#d699b6", operator: "#e69875", tag: "#7fbbb3", attribute: "#dbbc7f",
      regexp: "#83c092", escape: "#e69875", heading: "#a7c080", link: "#7fbbb3", meta: "#859289",
    },
  },
  "kanagawa": {
    ui: {
      bg: "#1f1f28", panel: "#16161d", panel2: "#2a2a37", border: "#363646",
      text: "#dcd7ba", text2: "#c8c093", text3: "#727169", sel: "#2d4f67",
      accent: "#7e9cd8", accent2: "#957fb8", green: "#98bb6c", red: "#c34043",
      blue: "#7e9cd8", purple: "#957fb8", teal: "#7aa89f", yellow: "#e6c384",
    },
    xterm: {
      background: "#1f1f28", foreground: "#dcd7ba", cursor: "#c8c093",
      cursorAccent: "#1f1f28", selectionBackground: "#2d4f67",
      black: "#16161d", red: "#c34043", green: "#76946a", yellow: "#c0a36e",
      blue: "#7e9cd8", magenta: "#957fb8", cyan: "#6a9589", white: "#c8c093",
      brightBlack: "#727169", brightRed: "#e82424", brightGreen: "#98bb6c",
      brightYellow: "#e6c384", brightBlue: "#7fb4ca", brightMagenta: "#938aa9",
      brightCyan: "#7aa89f", brightWhite: "#dcd7ba",
    },
    syntax: {
      comment: "#727169", keyword: "#957fb8", string: "#98bb6c", number: "#d27e99",
      type: "#7aa89f", function: "#7e9cd8", property: "#dcd7ba", variable: "#dcd7ba",
      constant: "#ffa066", operator: "#c0a36e", tag: "#7e9cd8", attribute: "#e6c384",
      regexp: "#98bb6c", escape: "#ffa066", heading: "#7e9cd8", link: "#7fb4ca", meta: "#727169",
    },
  },
  "night-owl": {
    ui: {
      bg: "#011627", panel: "#01111d", panel2: "#0e293f", border: "#1d3b53",
      text: "#d6deeb", text2: "#aeb9c9", text3: "#637777", sel: "#1d3b53",
      accent: "#82aaff", accent2: "#c792ea", green: "#addb67", red: "#ef5350",
      blue: "#82aaff", purple: "#c792ea", teal: "#7fdbca", yellow: "#ecc48d",
    },
    xterm: {
      background: "#011627", foreground: "#d6deeb", cursor: "#80a4c2",
      cursorAccent: "#011627", selectionBackground: "#1d3b53",
      black: "#011627", red: "#ef5350", green: "#addb67", yellow: "#c5e478",
      blue: "#82aaff", magenta: "#c792ea", cyan: "#21c7a8", white: "#ffffff",
      brightBlack: "#575656", brightRed: "#ef5350", brightGreen: "#22da6e",
      brightYellow: "#ffeb95", brightBlue: "#82aaff", brightMagenta: "#c792ea",
      brightCyan: "#7fdbca", brightWhite: "#ffffff",
    },
    syntax: {
      comment: "#637777", keyword: "#c792ea", string: "#ecc48d", number: "#f78c6c",
      type: "#ffcb8b", function: "#82aaff", property: "#80cbc4", variable: "#d6deeb",
      constant: "#f78c6c", operator: "#7fdbca", tag: "#7fdbca", attribute: "#addb67",
      regexp: "#5ca7e4", escape: "#f78c6c", heading: "#82aaff", link: "#80cbc4", meta: "#637777",
    },
  },
  "tokyo-night-storm": {
    ui: {
      bg: "#24283b", panel: "#1f2335", panel2: "#2a2e42", border: "#32344a",
      text: "#c0caf5", text2: "#a9b1d6", text3: "#565f89", sel: "#2e3c64",
      accent: "#7aa2f7", accent2: "#bb9af7", green: "#9ece6a", red: "#f7768e",
      blue: "#7aa2f7", purple: "#bb9af7", teal: "#7dcfff", yellow: "#e0af68",
    },
    xterm: {
      background: "#24283b", foreground: "#c0caf5", cursor: "#c0caf5",
      cursorAccent: "#24283b", selectionBackground: "#2e3c64",
      black: "#1d202f", red: "#f7768e", green: "#9ece6a", yellow: "#e0af68",
      blue: "#7aa2f7", magenta: "#bb9af7", cyan: "#7dcfff", white: "#a9b1d6",
      brightBlack: "#414868", brightRed: "#f7768e", brightGreen: "#9ece6a",
      brightYellow: "#e0af68", brightBlue: "#7aa2f7", brightMagenta: "#bb9af7",
      brightCyan: "#7dcfff", brightWhite: "#c0caf5",
    },
    syntax: {
      comment: "#565f89", keyword: "#bb9af7", string: "#9ece6a", number: "#ff9e64",
      type: "#2ac3de", function: "#7aa2f7", property: "#7dcfff", variable: "#c0caf5",
      constant: "#ff9e64", operator: "#89ddff", tag: "#f7768e", attribute: "#bb9af7",
      regexp: "#b4f9f8", escape: "#89ddff", heading: "#7aa2f7", link: "#7dcfff", meta: "#565f89",
    },
  },
  "ayu-dark": {
    ui: {
      bg: "#0b0e14", panel: "#0d1017", panel2: "#1c2230", border: "#1c2733",
      text: "#bfbdb6", text2: "#acaca0", text3: "#565b66", sel: "#224a5c",
      accent: "#ffb454", accent2: "#59c2ff", green: "#aad94c", red: "#f07178",
      blue: "#59c2ff", purple: "#d2a6ff", teal: "#95e6cb", yellow: "#ffb454",
    },
    xterm: {
      background: "#0b0e14", foreground: "#bfbdb6", cursor: "#ffb454",
      cursorAccent: "#0b0e14", selectionBackground: "#224a5c",
      black: "#01060e", red: "#ea6c73", green: "#91b362", yellow: "#f9af4f",
      blue: "#53bdfa", magenta: "#fae994", cyan: "#90e1c6", white: "#c7c7c7",
      brightBlack: "#686868", brightRed: "#f07178", brightGreen: "#c2d94c",
      brightYellow: "#ffb454", brightBlue: "#59c2ff", brightMagenta: "#d2a6ff",
      brightCyan: "#95e6cb", brightWhite: "#ffffff",
    },
    syntax: {
      comment: "#565b66", keyword: "#ff8f40", string: "#aad94c", number: "#ffb454",
      type: "#59c2ff", function: "#ffb454", property: "#bfbdb6", variable: "#bfbdb6",
      constant: "#d2a6ff", operator: "#f29668", tag: "#39bae6", attribute: "#ffb454",
      regexp: "#95e6cb", escape: "#95e6cb", heading: "#ffb454", link: "#39bae6", meta: "#565b66",
    },
  },
  "one-light": {
    ui: {
      bg: "#fafafa", panel: "#f0f0f0", panel2: "#e5e5e6", border: "#d4d4d4",
      text: "#383a42", text2: "#4f525d", text3: "#a0a1a7", sel: "#e5e5e6",
      accent: "#4078f2", accent2: "#a626a4", green: "#50a14f", red: "#e45649",
      blue: "#4078f2", purple: "#a626a4", teal: "#0184bc", yellow: "#c18401",
    },
    xterm: {
      background: "#fafafa", foreground: "#383a42", cursor: "#4078f2",
      cursorAccent: "#fafafa", selectionBackground: "#e5e5e6",
      black: "#383a42", red: "#e45649", green: "#50a14f", yellow: "#c18401",
      blue: "#4078f2", magenta: "#a626a4", cyan: "#0184bc", white: "#a0a1a7",
      brightBlack: "#4f525d", brightRed: "#e45649", brightGreen: "#50a14f",
      brightYellow: "#c18401", brightBlue: "#4078f2", brightMagenta: "#a626a4",
      brightCyan: "#0184bc", brightWhite: "#fafafa",
    },
    syntax: {
      comment: "#a0a1a7", keyword: "#a626a4", string: "#50a14f", number: "#986801",
      type: "#c18401", function: "#4078f2", property: "#e45649", variable: "#383a42",
      constant: "#986801", operator: "#0184bc", tag: "#e45649", attribute: "#986801",
      regexp: "#50a14f", escape: "#0184bc", heading: "#e45649", link: "#4078f2", meta: "#a0a1a7",
    },
  },
  "oxocarbon": {
    ui: {
      bg: "#161616", panel: "#262626", panel2: "#393939", border: "#393939",
      text: "#f2f4f8", text2: "#dde1e6", text3: "#525252", sel: "#393939",
      accent: "#33b1ff", accent2: "#ee5396", green: "#42be65", red: "#ee5396",
      blue: "#33b1ff", purple: "#be95ff", teal: "#3ddbd9", yellow: "#fae588",
    },
    xterm: {
      background: "#161616", foreground: "#f2f4f8", cursor: "#f2f4f8",
      cursorAccent: "#161616", selectionBackground: "#393939",
      black: "#262626", red: "#ff7eb6", green: "#42be65", yellow: "#ffe97b",
      blue: "#33b1ff", magenta: "#be95ff", cyan: "#3ddbd9", white: "#dde1e6",
      brightBlack: "#525252", brightRed: "#ff7eb6", brightGreen: "#42be65",
      brightYellow: "#ffe97b", brightBlue: "#33b1ff", brightMagenta: "#be95ff",
      brightCyan: "#3ddbd9", brightWhite: "#ffffff",
    },
    syntax: {
      comment: "#525252", keyword: "#ff7eb6", string: "#42be65", number: "#be95ff",
      type: "#ee5396", function: "#3ddbd9", property: "#33b1ff", variable: "#f2f4f8",
      constant: "#be95ff", operator: "#33b1ff", tag: "#ee5396", attribute: "#08bdba",
      regexp: "#42be65", escape: "#be95ff", heading: "#33b1ff", link: "#78a9ff", meta: "#525252",
    },
  },
  "gruvbox-material": {
    ui: {
      bg: "#282828", panel: "#32302f", panel2: "#3c3836", border: "#504945",
      text: "#d4be98", text2: "#ddc7a1", text3: "#7c6f64", sel: "#45403d",
      accent: "#7daea3", accent2: "#e78a4e", green: "#a9b665", red: "#ea6962",
      blue: "#7daea3", purple: "#d3869b", teal: "#89b482", yellow: "#d8a657",
    },
    xterm: {
      background: "#282828", foreground: "#d4be98", cursor: "#d4be98",
      cursorAccent: "#282828", selectionBackground: "#45403d",
      black: "#32302f", red: "#ea6962", green: "#a9b665", yellow: "#d8a657",
      blue: "#7daea3", magenta: "#d3869b", cyan: "#89b482", white: "#d4be98",
      brightBlack: "#5b534d", brightRed: "#ea6962", brightGreen: "#a9b665",
      brightYellow: "#d8a657", brightBlue: "#7daea3", brightMagenta: "#d3869b",
      brightCyan: "#89b482", brightWhite: "#ddc7a1",
    },
    syntax: {
      comment: "#7c6f64", keyword: "#ea6962", string: "#a9b665", number: "#d3869b",
      type: "#d8a657", function: "#a9b665", property: "#7daea3", variable: "#d4be98",
      constant: "#d3869b", operator: "#e78a4e", tag: "#89b482", attribute: "#d8a657",
      regexp: "#a9b665", escape: "#e78a4e", heading: "#a9b665", link: "#7daea3", meta: "#7c6f64",
    },
  },
};

export const THEME_LABELS: Record<string, string> = {
  "anvil-dark": "Anvil Dark",
  "anvil-light": "Anvil Light",
  "solarized-dark": "Solarized Dark",
  "solarized-light": "Solarized Light",
  "amber-dark": "Amber Dark",
  "tokyo-night": "Tokyo Night",
  "gruvbox-dark": "Gruvbox Dark",
  "catppuccin-mocha": "Catppuccin Mocha",
  "nord": "Nord",
  "dracula": "Dracula",
  "rose-pine": "Rosé Pine",
  "rose-pine-dawn": "Rosé Pine Dawn",
  "gruvbox-light": "Gruvbox Light",
  "one-dark": "One Dark",
  "github-dark": "GitHub Dark",
  "github-light": "GitHub Light",
  "catppuccin-macchiato": "Catppuccin Macchiato",
  "catppuccin-latte": "Catppuccin Latte",
  "ayu-mirage": "Ayu Mirage",
  "monokai": "Monokai",
  "everforest": "Everforest",
  "kanagawa": "Kanagawa",
  "night-owl": "Night Owl",
  "tokyo-night-storm": "Tokyo Night Storm",
  "ayu-dark": "Ayu Dark",
  "one-light": "One Light",
  "oxocarbon": "Oxocarbon",
  "gruvbox-material": "Gruvbox Material",
};

export function themeLabel(key: string): string {
  return THEME_LABELS[key] ?? key.replace(/-/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

export const THEME_KEYS = Object.keys(themes);
export const isLight = (name: string) => name.includes("light") || name.includes("dawn");
export const LIGHT_THEMES = THEME_KEYS.filter(isLight);
export const DARK_THEMES = THEME_KEYS.filter((n) => !isLight(n));

export const activeTheme = writable<string>("anvil-dark");
export const systemMode = writable<boolean>(false);
export const systemLight = writable<string>("anvil-light");
export const systemDark = writable<string>("anvil-dark");

function read<T>(s: import("svelte/store").Readable<T>): T {
  let v!: T;
  s.subscribe((x) => (v = x))();
  return v;
}

// Apply a theme's palette to the CSS vars + re-layer any custom-color overrides.
function applyVars(name: string): void {
  const t = themes[name];
  if (!t || typeof document === "undefined") return;
  const root = document.documentElement.style;
  for (const [k, v] of Object.entries(t.ui)) root.setProperty(`--${k}`, v);
  // Custom overrides win over the base theme.
  try {
    const ov = JSON.parse(localStorage.getItem("anvil-custom-theme") || "{}");
    for (const [k, v] of Object.entries(ov)) root.setProperty(`--${k}`, v as string);
  } catch { /* ignore */ }
  document.documentElement.style.colorScheme = isLight(name) ? "light" : "dark";
  activeTheme.set(name);
}

const prefersLight = () =>
  typeof window !== "undefined" && window.matchMedia("(prefers-color-scheme: light)").matches;

function applySystem(): void {
  applyVars(prefersLight() ? read(systemLight) : read(systemDark));
}

/** Manual theme pick — turns system mode off. */
export function applyTheme(name: string): void {
  if (read(systemMode)) { systemMode.set(false); localStorage.setItem("anvil-system-mode", "0"); }
  applyVars(name);
  localStorage.setItem("anvil-theme", name);
}

export function setSystemMode(on: boolean): void {
  systemMode.set(on);
  localStorage.setItem("anvil-system-mode", on ? "1" : "0");
  if (on) applySystem();
  else applyVars(localStorage.getItem("anvil-theme") ?? "anvil-dark");
}
export function setSystemPair(light: string, dark: string): void {
  systemLight.set(light); systemDark.set(dark);
  localStorage.setItem("anvil-system-light", light);
  localStorage.setItem("anvil-system-dark", dark);
  if (read(systemMode)) applySystem();
}

let mqBound = false;
export function initTheme(): void {
  systemLight.set(localStorage.getItem("anvil-system-light") ?? "anvil-light");
  systemDark.set(localStorage.getItem("anvil-system-dark") ?? "anvil-dark");
  const sys = localStorage.getItem("anvil-system-mode") === "1";
  systemMode.set(sys);
  if (sys) applySystem();
  else applyVars(localStorage.getItem("anvil-theme") ?? "anvil-dark");
  if (!mqBound && typeof window !== "undefined") {
    mqBound = true;
    window.matchMedia("(prefers-color-scheme: light)").addEventListener("change", () => {
      if (read(systemMode)) applySystem();
    });
  }
}

export function cycleTheme(): void {
  const idx = THEME_KEYS.indexOf(read(activeTheme));
  applyTheme(THEME_KEYS[(idx + 1) % THEME_KEYS.length]);
}
