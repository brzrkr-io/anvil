-- anvil — Neovim colorscheme matching Anvil's Mineral palette (BRAND.md).
-- Tokens are copied verbatim from crates-free src/render/theme.zig so a buffer's
-- Normal background equals the terminal background exactly: zero seam.
--
-- Variant selection, in order: an explicit `:set background`, else the
-- ANVIL_THEME env Anvil exports at spawn (mineral-dark / mineral-light),
-- else dark. Pick with `:colorscheme anvil`.

local dark = {
  bg = "#0e0b0a", fg = "#d8cfc8", bar = "#1c1614", sep = "#3d2f27",
  sel_bg = "#3a2820", sel_fg = "#f0ebe4",
  black = "#1c1614", red = "#b53a2e", green = "#4a8c52", yellow = "#b88220",
  blue = "#5272a0", magenta = "#8c5fa0", cyan = "#c2614a", white = "#d8cfc8",
  br_black = "#503e34", br_red = "#d45a44", br_green = "#62a86a", br_yellow = "#d4a030",
  br_blue = "#6882b8", br_magenta = "#a878c0", br_cyan = "#d4733e", br_white = "#f0ebe4",
}

local light = {
  bg = "#f2ece4", fg = "#302520", bar = "#fdf6ee", sep = "#cbbfb4",
  sel_bg = "#e0d4c8", sel_fg = "#140e0a",
  black = "#140e0a", red = "#a03025", green = "#2e6c38", yellow = "#8c6214",
  blue = "#3a5888", magenta = "#6c4880", cyan = "#a84c38", white = "#5c4e44",
  br_black = "#9c8878", br_red = "#c5462a", br_green = "#4a8c52", br_yellow = "#b88220",
  br_blue = "#5272a0", br_magenta = "#8c5fa0", br_cyan = "#c2614a", br_white = "#3d2e26",
}

local function variant()
  if vim.g.anvil_background == "light" or vim.g.anvil_background == "dark" then
    return vim.g.anvil_background
  end
  local env = vim.env.ANVIL_THEME
  if env == "mineral-light" then return "light" end
  if env == "mineral-dark" then return "dark" end
  return vim.o.background == "light" and "light" or "dark"
end

local v = variant()
local p = v == "light" and light or dark

vim.cmd("highlight clear")
if vim.fn.exists("syntax_on") == 1 then vim.cmd("syntax reset") end
vim.o.termguicolors = true
vim.o.background = v
vim.g.colors_name = "anvil"

-- Map Anvil's terminal ANSI palette so :terminal output matches too.
local ansi = { p.black, p.red, p.green, p.yellow, p.blue, p.magenta, p.cyan, p.white,
  p.br_black, p.br_red, p.br_green, p.br_yellow, p.br_blue, p.br_magenta, p.br_cyan, p.br_white }
for i = 0, 15 do vim.g["terminal_color_" .. i] = ansi[i + 1] end

local muted = v == "light" and p.white or p.br_black -- secondary text
local function hl(group, spec) vim.api.nvim_set_hl(0, group, spec) end

local groups = {
  -- editor surface
  Normal = { fg = p.fg, bg = p.bg },
  NormalNC = { fg = p.fg, bg = p.bg },
  NormalFloat = { fg = p.fg, bg = p.bar },
  FloatBorder = { fg = p.sep, bg = p.bar },
  FloatTitle = { fg = p.br_cyan, bg = p.bar, bold = true },
  ColorColumn = { bg = p.bar },
  Cursor = { fg = p.bg, bg = p.fg },
  CursorLine = { bg = p.bar },
  CursorColumn = { bg = p.bar },
  CursorLineNr = { fg = p.fg, bold = true },
  LineNr = { fg = muted },
  SignColumn = { bg = p.bg },
  Folded = { fg = muted, bg = p.bar },
  FoldColumn = { fg = muted },
  WinSeparator = { fg = p.sep },
  VertSplit = { fg = p.sep },
  Visual = { bg = p.sel_bg, fg = p.sel_fg },
  Search = { fg = p.bg, bg = p.br_yellow },
  IncSearch = { fg = p.bg, bg = p.br_cyan },
  CurSearch = { fg = p.bg, bg = p.br_cyan },
  MatchParen = { fg = p.br_cyan, bold = true },
  Whitespace = { fg = p.sep },
  NonText = { fg = p.sep },
  EndOfBuffer = { fg = p.bg },
  Directory = { fg = p.br_blue },
  Title = { fg = p.br_cyan, bold = true },
  WinBar = { fg = p.fg, bg = p.bar },
  WinBarNC = { fg = muted, bg = p.bar },

  -- statusline / tabline / popup menu
  StatusLine = { fg = p.fg, bg = p.bar },
  StatusLineNC = { fg = muted, bg = p.bar },
  TabLine = { fg = muted, bg = p.bar },
  TabLineSel = { fg = p.fg, bg = p.bg, bold = true },
  TabLineFill = { bg = p.bar },
  Pmenu = { fg = p.fg, bg = p.bar },
  PmenuSel = { fg = p.sel_fg, bg = p.sel_bg },
  PmenuSbar = { bg = p.bar },
  PmenuThumb = { bg = p.sep },

  -- messages
  ErrorMsg = { fg = p.br_red },
  WarningMsg = { fg = p.br_yellow },
  ModeMsg = { fg = p.fg },
  MoreMsg = { fg = p.br_green },
  Question = { fg = p.br_cyan },

  -- syntax
  Comment = { fg = muted, italic = true },
  Constant = { fg = p.br_cyan },
  String = { fg = p.green },
  Character = { fg = p.green },
  Number = { fg = p.br_cyan },
  Boolean = { fg = p.br_cyan },
  Float = { fg = p.br_cyan },
  Identifier = { fg = p.fg },
  Function = { fg = p.br_blue },
  Statement = { fg = p.br_magenta },
  Conditional = { fg = p.br_magenta },
  Repeat = { fg = p.br_magenta },
  Label = { fg = p.br_magenta },
  Operator = { fg = p.cyan },
  Keyword = { fg = p.br_magenta },
  Exception = { fg = p.br_red },
  PreProc = { fg = p.br_red },
  Include = { fg = p.br_red },
  Define = { fg = p.br_red },
  Macro = { fg = p.br_red },
  Type = { fg = p.br_yellow },
  StorageClass = { fg = p.br_yellow },
  Structure = { fg = p.br_yellow },
  Typedef = { fg = p.br_yellow },
  Special = { fg = p.br_cyan },
  SpecialChar = { fg = p.br_cyan },
  Delimiter = { fg = p.fg },
  Todo = { fg = p.bg, bg = p.br_yellow, bold = true },
  Error = { fg = p.br_red },
  Underlined = { fg = p.br_blue, underline = true },

  -- diagnostics
  DiagnosticError = { fg = p.br_red },
  DiagnosticWarn = { fg = p.br_yellow },
  DiagnosticInfo = { fg = p.br_blue },
  DiagnosticHint = { fg = p.br_cyan },
  DiagnosticOk = { fg = p.br_green },
  DiagnosticUnderlineError = { sp = p.br_red, undercurl = true },
  DiagnosticUnderlineWarn = { sp = p.br_yellow, undercurl = true },
  DiagnosticUnderlineInfo = { sp = p.br_blue, undercurl = true },
  DiagnosticUnderlineHint = { sp = p.br_cyan, undercurl = true },

  -- diff / git
  DiffAdd = { bg = v == "light" and "#dceede" or "#16241a" },
  DiffChange = { bg = v == "light" and "#e6e9d6" or "#23210f" },
  DiffDelete = { bg = v == "light" and "#f0dcd9" or "#241413" },
  DiffText = { bg = v == "light" and "#cfe0cf" or "#22381f" },
  Added = { fg = p.green },
  Changed = { fg = p.yellow },
  Removed = { fg = p.red },
  GitSignsAdd = { fg = p.green },
  GitSignsChange = { fg = p.yellow },
  GitSignsDelete = { fg = p.red },

  -- spell
  SpellBad = { sp = p.br_red, undercurl = true },
  SpellCap = { sp = p.br_yellow, undercurl = true },
  SpellRare = { sp = p.br_magenta, undercurl = true },
  SpellLocal = { sp = p.br_cyan, undercurl = true },
}

for group, spec in pairs(groups) do hl(group, spec) end

-- Treesitter captures → base groups (covers most languages).
local links = {
  ["@comment"] = "Comment",
  ["@string"] = "String",
  ["@string.escape"] = "SpecialChar",
  ["@character"] = "Character",
  ["@number"] = "Number",
  ["@boolean"] = "Boolean",
  ["@float"] = "Float",
  ["@constant"] = "Constant",
  ["@constant.builtin"] = "Constant",
  ["@constant.macro"] = "Macro",
  ["@variable"] = "Identifier",
  ["@variable.builtin"] = "Special",
  ["@variable.parameter"] = "Identifier",
  ["@field"] = "Identifier",
  ["@property"] = "Identifier",
  ["@function"] = "Function",
  ["@function.builtin"] = "Function",
  ["@function.call"] = "Function",
  ["@function.macro"] = "Macro",
  ["@method"] = "Function",
  ["@method.call"] = "Function",
  ["@constructor"] = "Type",
  ["@keyword"] = "Keyword",
  ["@keyword.function"] = "Keyword",
  ["@keyword.operator"] = "Operator",
  ["@keyword.return"] = "Keyword",
  ["@conditional"] = "Conditional",
  ["@repeat"] = "Repeat",
  ["@exception"] = "Exception",
  ["@operator"] = "Operator",
  ["@type"] = "Type",
  ["@type.builtin"] = "Type",
  ["@type.definition"] = "Typedef",
  ["@namespace"] = "Type",
  ["@include"] = "Include",
  ["@preproc"] = "PreProc",
  ["@punctuation.delimiter"] = "Delimiter",
  ["@punctuation.bracket"] = "Delimiter",
  ["@punctuation.special"] = "Special",
  ["@tag"] = "Keyword",
  ["@tag.attribute"] = "Identifier",
  ["@tag.delimiter"] = "Delimiter",
  ["@text.title"] = "Title",
  ["@text.literal"] = "String",
  ["@text.uri"] = "Underlined",
  ["@label"] = "Label",
  -- nvim 0.10 capture renames
  ["@string.special.url"] = "Underlined",
  ["@markup.heading"] = "Title",
  ["@markup.raw"] = "String",
  ["@markup.link"] = "Underlined",
  ["@module"] = "Type",
  ["@lsp.type.class"] = "Type",
  ["@lsp.type.function"] = "Function",
  ["@lsp.type.method"] = "Function",
  ["@lsp.type.keyword"] = "Keyword",
  ["@lsp.type.variable"] = "Identifier",
  ["@lsp.type.property"] = "Identifier",
}
for capture, target in pairs(links) do hl(capture, { link = target }) end
