import { describe, it, expect } from "vitest";
import { cmLang } from "./cm-lang";

// cmLang returns Extension[]. We verify non-empty (language loaded) vs empty (plain text fallback).
// We don't assert on internal CodeMirror details — just that the right parser is returned.

describe("cmLang — TypeScript / JavaScript family", () => {
  it("returns an extension for .ts files", () => {
    expect(cmLang("src/app.ts")).toHaveLength(1);
  });
  it("returns an extension for .tsx files", () => {
    expect(cmLang("App.tsx")).toHaveLength(1);
  });
  it("returns an extension for .jsx files", () => {
    expect(cmLang("Comp.jsx")).toHaveLength(1);
  });
  it("returns an extension for .js files", () => {
    expect(cmLang("util.js")).toHaveLength(1);
  });
  it("returns an extension for .mjs files", () => {
    expect(cmLang("util.mjs")).toHaveLength(1);
  });
  it("returns an extension for .cjs files", () => {
    expect(cmLang("util.cjs")).toHaveLength(1);
  });
});

describe("cmLang — web stack", () => {
  it("returns an extension for .json files", () => {
    expect(cmLang("package.json")).toHaveLength(1);
  });
  it("returns an extension for .css files", () => {
    expect(cmLang("styles.css")).toHaveLength(1);
  });
  it("returns an extension for .scss files", () => {
    expect(cmLang("app.scss")).toHaveLength(1);
  });
  it("returns an extension for .less files", () => {
    expect(cmLang("app.less")).toHaveLength(1);
  });
  it("returns an extension for .html files", () => {
    expect(cmLang("index.html")).toHaveLength(1);
  });
  it("returns an extension for .htm files", () => {
    expect(cmLang("index.htm")).toHaveLength(1);
  });
  it("returns an extension for .svelte files", () => {
    expect(cmLang("App.svelte")).toHaveLength(1);
  });
  it("returns an extension for .vue files", () => {
    expect(cmLang("App.vue")).toHaveLength(1);
  });
});

describe("cmLang — docs and data", () => {
  it("returns an extension for .md files", () => {
    expect(cmLang("README.md")).toHaveLength(1);
  });
  it("returns an extension for .markdown files", () => {
    expect(cmLang("notes.markdown")).toHaveLength(1);
  });
  it("returns an extension for .yaml files", () => {
    expect(cmLang("config.yaml")).toHaveLength(1);
  });
  it("returns an extension for .yml files", () => {
    expect(cmLang("ci.yml")).toHaveLength(1);
  });
  it("returns an extension for .xml files", () => {
    expect(cmLang("pom.xml")).toHaveLength(1);
  });
  it("returns an extension for .sql files", () => {
    expect(cmLang("query.sql")).toHaveLength(1);
  });
});

describe("cmLang — systems languages", () => {
  it("returns an extension for .rs files", () => {
    expect(cmLang("main.rs")).toHaveLength(1);
  });
  it("returns an extension for .go files", () => {
    expect(cmLang("main.go")).toHaveLength(1);
  });
  it("returns an extension for .py files", () => {
    expect(cmLang("script.py")).toHaveLength(1);
  });
  it("returns an extension for .c files", () => {
    expect(cmLang("main.c")).toHaveLength(1);
  });
  it("returns an extension for .h files", () => {
    expect(cmLang("header.h")).toHaveLength(1);
  });
  it("returns an extension for .cpp files", () => {
    expect(cmLang("app.cpp")).toHaveLength(1);
  });
  it("returns an extension for .hpp files", () => {
    expect(cmLang("app.hpp")).toHaveLength(1);
  });
  it("returns an extension for .cc files", () => {
    expect(cmLang("app.cc")).toHaveLength(1);
  });
  it("returns an extension for .cxx files", () => {
    expect(cmLang("app.cxx")).toHaveLength(1);
  });
});

describe("cmLang — shell and config", () => {
  it("returns an extension for .sh files", () => {
    expect(cmLang("deploy.sh")).toHaveLength(1);
  });
  it("returns an extension for .bash files", () => {
    expect(cmLang("run.bash")).toHaveLength(1);
  });
  it("returns an extension for .zsh files", () => {
    expect(cmLang("run.zsh")).toHaveLength(1);
  });
  it("returns an extension for .fish files", () => {
    expect(cmLang("run.fish")).toHaveLength(1);
  });
  it("returns an extension for .toml files", () => {
    expect(cmLang("Cargo.toml")).toHaveLength(1);
  });
  it("returns an extension for .ini files", () => {
    expect(cmLang("config.ini")).toHaveLength(1);
  });
  it("returns an extension for .conf files", () => {
    expect(cmLang("nginx.conf")).toHaveLength(1);
  });
  it("returns an extension for .properties files", () => {
    expect(cmLang("app.properties")).toHaveLength(1);
  });
  it("returns an extension for .env files", () => {
    expect(cmLang(".env")).toHaveLength(1);
  });
  it("returns an extension for .lua files", () => {
    expect(cmLang("init.lua")).toHaveLength(1);
  });
  it("returns an extension for .rb files", () => {
    expect(cmLang("app.rb")).toHaveLength(1);
  });
});

describe("cmLang — special filenames", () => {
  it("returns an extension for Dockerfile (exact filename match)", () => {
    expect(cmLang("Dockerfile")).toHaveLength(1);
  });
  it("matches Dockerfile case-insensitively", () => {
    expect(cmLang("dockerfile")).toHaveLength(1);
  });
  it("returns empty array for Makefile (no language support)", () => {
    expect(cmLang("Makefile")).toHaveLength(0);
  });
});

describe("cmLang — unknown / plain text fallback", () => {
  it("returns empty array for an unknown extension", () => {
    expect(cmLang("file.unknownxyz")).toHaveLength(0);
  });
  it("returns empty array for a file with no extension", () => {
    expect(cmLang("justanamenoext")).toHaveLength(0);
  });
});
