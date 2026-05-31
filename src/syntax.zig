const std = @import("std");

pub const Role = enum { text, keyword, string, number, comment, type, punct };

pub const Token = struct { start: usize, len: usize, role: Role };

pub const Lang = enum { zig, toml, markdown, sh, lua, generic, unknown };

pub fn detect(path: []const u8) Lang {
    const ext_start = std.mem.lastIndexOfScalar(u8, path, '.') orelse return .unknown;
    const ext = path[ext_start..];
    if (std.mem.eql(u8, ext, ".zig")) return .zig;
    if (std.mem.eql(u8, ext, ".toml")) return .toml;
    if (std.mem.eql(u8, ext, ".md") or std.mem.eql(u8, ext, ".markdown")) return .markdown;
    if (std.mem.eql(u8, ext, ".sh") or std.mem.eql(u8, ext, ".bash") or std.mem.eql(u8, ext, ".zsh")) return .sh;
    if (std.mem.eql(u8, ext, ".lua")) return .lua;
    if (std.mem.eql(u8, ext, ".c") or std.mem.eql(u8, ext, ".h") or
        std.mem.eql(u8, ext, ".cpp") or std.mem.eql(u8, ext, ".rs") or
        std.mem.eql(u8, ext, ".go") or std.mem.eql(u8, ext, ".js") or
        std.mem.eql(u8, ext, ".ts") or std.mem.eql(u8, ext, ".py"))
        return .generic;
    return .unknown;
}

const zig_keywords = [_][]const u8{
    "const", "var",         "fn",        "pub",      "return",    "if",       "else",   "while",
    "for",   "switch",      "break",     "continue", "defer",     "errdefer", "try",    "catch",
    "error", "union",       "struct",    "enum",     "packed",    "extern",   "export", "inline",
    "test",  "unreachable", "comptime",  "anytype",  "anyopaque", "bool",     "void",   "noreturn",
    "type",  "null",        "undefined", "true",     "false",     "and",      "or",     "orelse",
    "usize", "isize",       "u8",        "u16",      "u32",       "u64",      "u128",   "i8",
    "i16",   "i32",         "i64",       "i128",     "f16",       "f32",      "f64",    "f128",
    "u21",   "u1",
};

const toml_keywords = [_][]const u8{ "true", "false" };

const sh_keywords = [_][]const u8{
    "if",    "then",     "else", "elif",     "fi",     "for",   "while",  "do",     "done",
    "case",  "esac",     "in",   "function", "return", "local", "export", "source", "readonly",
    "break", "continue",
};

const lua_keywords = [_][]const u8{
    "and",      "break",  "do",   "else", "elseif", "end",   "false", "for",
    "function", "goto",   "if",   "in",   "local",  "nil",   "not",   "or",
    "repeat",   "return", "then", "true", "until",  "while",
};

const generic_keywords = [_][]const u8{
    "if",       "else", "while",   "for",     "return",    "break",     "continue",
    "switch",   "case", "default", "do",      "const",     "let",       "var",
    "function", "fn",   "pub",     "private", "protected", "class",     "struct",
    "enum",     "void", "int",     "char",    "bool",      "true",      "false",
    "null",     "nil",  "self",    "this",    "new",       "delete",    "import",
    "export",   "from", "use",     "mod",     "type",      "interface", "impl",
    "trait",
};

fn isAlpha(c: u8) bool {
    return (c >= 'a' and c <= 'z') or (c >= 'A' and c <= 'Z') or c == '_';
}

fn isAlnum(c: u8) bool {
    return isAlpha(c) or (c >= '0' and c <= '9');
}

fn isDigit(c: u8) bool {
    return c >= '0' and c <= '9';
}

fn matchKeyword(word: []const u8, keywords: []const []const u8) bool {
    for (keywords) |kw| {
        if (std.mem.eql(u8, word, kw)) return true;
    }
    return false;
}

fn isTypeIdent(word: []const u8) bool {
    if (word.len == 0) return false;
    const first = word[0];
    return first >= 'A' and first <= 'Z';
}

pub fn tokenizeLine(lang: Lang, line: []const u8, out: []Token) usize {
    if (lang == .unknown) {
        if (line.len > 0 and out.len > 0) {
            out[0] = .{ .start = 0, .len = line.len, .role = .text };
            return 1;
        }
        return 0;
    }

    var n: usize = 0;
    var i: usize = 0;

    const emit = struct {
        fn call(tokens: []Token, count: *usize, start: usize, len: usize, role: Role) void {
            if (count.* < tokens.len and len > 0) {
                tokens[count.*] = .{ .start = start, .len = len, .role = role };
                count.* += 1;
            }
        }
    }.call;

    while (i < line.len) {
        const ch = line[i];

        // Line comments
        const is_comment = switch (lang) {
            .zig => i + 1 < line.len and ch == '/' and line[i + 1] == '/',
            .toml => ch == '#',
            .sh => ch == '#',
            .lua => i + 1 < line.len and ch == '-' and line[i + 1] == '-',
            .generic => i + 1 < line.len and ch == '/' and line[i + 1] == '/',
            .markdown => false,
            .unknown => false,
        };
        if (is_comment) {
            emit(out, &n, i, line.len - i, .comment);
            break;
        }

        // Markdown headings
        if (lang == .markdown and ch == '#') {
            var j = i;
            while (j < line.len and line[j] == '#') j += 1;
            if (j < line.len and line[j] == ' ') {
                emit(out, &n, i, line.len - i, .keyword);
                break;
            }
        }

        // Strings (single or double quote)
        if (ch == '"' or ch == '\'') {
            const q = ch;
            var j = i + 1;
            while (j < line.len and line[j] != q) {
                if (line[j] == '\\') j += 1;
                j += 1;
            }
            if (j < line.len) j += 1;
            emit(out, &n, i, j - i, .string);
            i = j;
            continue;
        }

        // Numbers (integer and float, including 0x hex)
        if (isDigit(ch) or (ch == '-' and i + 1 < line.len and isDigit(line[i + 1]))) {
            var j = i;
            if (line[j] == '-') j += 1;
            if (j + 1 < line.len and line[j] == '0' and (line[j + 1] == 'x' or line[j + 1] == 'X')) {
                j += 2;
                while (j < line.len and (isAlnum(line[j]))) j += 1;
            } else {
                while (j < line.len and (isDigit(line[j]) or line[j] == '.' or line[j] == '_')) j += 1;
            }
            emit(out, &n, i, j - i, .number);
            i = j;
            continue;
        }

        // Identifiers and keywords
        if (isAlpha(ch)) {
            var j = i + 1;
            while (j < line.len and isAlnum(line[j])) j += 1;
            const word = line[i..j];
            const role: Role = blk: {
                const kws: []const []const u8 = switch (lang) {
                    .zig => &zig_keywords,
                    .toml => &toml_keywords,
                    .sh => &sh_keywords,
                    .lua => &lua_keywords,
                    .generic, .markdown => &generic_keywords,
                    .unknown => break :blk .text,
                };
                if (matchKeyword(word, kws)) break :blk .keyword;
                if (lang == .zig and isTypeIdent(word)) break :blk .type;
                break :blk .text;
            };
            emit(out, &n, i, j - i, role);
            i = j;
            continue;
        }

        // Punctuation (non-alphanumeric, non-space)
        if (ch != ' ' and ch != '\t') {
            emit(out, &n, i, 1, .punct);
        } else {
            emit(out, &n, i, 1, .text);
        }
        i += 1;
    }

    return n;
}

test "detect maps extensions correctly" {
    try std.testing.expectEqual(Lang.zig, detect("foo.zig"));
    try std.testing.expectEqual(Lang.toml, detect("config.toml"));
    try std.testing.expectEqual(Lang.markdown, detect("README.md"));
    try std.testing.expectEqual(Lang.sh, detect("run.sh"));
    try std.testing.expectEqual(Lang.lua, detect("init.lua"));
    try std.testing.expectEqual(Lang.unknown, detect("binary.exe"));
    try std.testing.expectEqual(Lang.unknown, detect("noext"));
    try std.testing.expectEqual(Lang.generic, detect("main.c"));
    try std.testing.expectEqual(Lang.generic, detect("lib.rs"));
}

test "tokenizeLine: zig keywords, strings, numbers, comments" {
    var toks: [64]Token = undefined;

    // keyword
    {
        const line = "const x = 42;";
        const n = tokenizeLine(.zig, line, &toks);
        try std.testing.expect(n > 0);
        try std.testing.expectEqual(Role.keyword, toks[0].role);
        try std.testing.expectEqualStrings("const", line[toks[0].start .. toks[0].start + toks[0].len]);
    }

    // string
    {
        const line = "var s = \"hello\";";
        const n = tokenizeLine(.zig, line, &toks);
        var found_string = false;
        for (toks[0..n]) |t| {
            if (t.role == .string) found_string = true;
        }
        try std.testing.expect(found_string);
    }

    // number
    {
        const line = "return 123;";
        const n = tokenizeLine(.zig, line, &toks);
        var found_num = false;
        for (toks[0..n]) |t| {
            if (t.role == .number) found_num = true;
        }
        try std.testing.expect(found_num);
    }

    // comment
    {
        const line = "// this is a comment";
        const n = tokenizeLine(.zig, line, &toks);
        try std.testing.expect(n > 0);
        try std.testing.expectEqual(Role.comment, toks[0].role);
        try std.testing.expectEqualStrings(line, line[toks[0].start .. toks[0].start + toks[0].len]);
    }

    // type (uppercase ident)
    {
        const line = "fn foo(x: MyType) void {}";
        const n = tokenizeLine(.zig, line, &toks);
        var found_type = false;
        for (toks[0..n]) |t| {
            if (t.role == .type and std.mem.eql(u8, "MyType", line[t.start .. t.start + t.len])) {
                found_type = true;
            }
        }
        try std.testing.expect(found_type);
    }
}

test "tokenizeLine: unknown lang returns whole line as text" {
    var toks: [64]Token = undefined;
    const line = "some arbitrary content 123";
    const n = tokenizeLine(.unknown, line, &toks);
    try std.testing.expectEqual(@as(usize, 1), n);
    try std.testing.expectEqual(Role.text, toks[0].role);
    try std.testing.expectEqual(@as(usize, 0), toks[0].start);
    try std.testing.expectEqual(line.len, toks[0].len);
}
