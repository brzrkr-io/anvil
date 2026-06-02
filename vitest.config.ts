import { defineConfig } from "vitest/config";
import { resolve } from "path";

export default defineConfig({
  resolve: {
    alias: {
      $lib: resolve(__dirname, "src/lib"),
    },
  },
  test: {
    environment: "happy-dom",
    // Node 26 defines localStorage/sessionStorage as globals set to undefined,
    // preventing happy-dom's populateGlobal from installing them. This setup
    // file re-installs them with a simple in-memory shim so tests can use
    // localStorage directly. See src/test-setup.ts.
    setupFiles: ["src/test-setup.ts"],
    include: ["src/**/*.test.ts"],
    coverage: {
      provider: "v8",
      reporter: ["text", "json-summary", "html"],
      include: ["src/lib/**/*.ts"],
      // #9 coverage target. Gate at 90% lines/statements so it can't regress;
      // branch is left ungated (DOM-glue branches inflate the denominator).
      thresholds: { lines: 90, statements: 90 },
      exclude: [
        "src/lib/**/*.test.ts",
        "src/lib/**/*.d.ts",
        // CodeMirror view-plugin / EditorView DOM glue — no extractable pure logic; covered by e2e, not unit-testable
        "src/lib/cm-ghost.ts",
        "src/lib/cm-lsp.ts",
        "src/lib/cm-color.ts",
        "src/lib/cm-theme.ts",
        // Trivial single-line writable store re-exports — no testable logic
        "src/lib/agent-seed.ts",
        "src/lib/editor-live.ts",
        "src/lib/terminal-open.ts",
      ],
    },
  },
});
