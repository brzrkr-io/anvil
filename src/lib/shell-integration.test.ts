import { describe, it, expect } from "vitest";
import { integrationFor } from "./shell-integration";

describe("integrationFor", () => {
  it("picks zsh by default and for zsh paths", () => {
    expect(integrationFor("/bin/zsh").shell).toBe("zsh");
    expect(integrationFor("").shell).toBe("zsh");
    expect(integrationFor("/bin/zsh").rc).toBe("~/.zshrc");
  });
  it("picks bash and fish from the shell path", () => {
    expect(integrationFor("/bin/bash").shell).toBe("bash");
    expect(integrationFor("/opt/homebrew/bin/fish").shell).toBe("fish");
  });
  it("every snippet emits the OSC 133 prompt marks", () => {
    for (const sh of ["/bin/zsh", "/bin/bash", "/usr/bin/fish"]) {
      const { snippet } = integrationFor(sh);
      expect(snippet).toContain("133;D");
      expect(snippet).toContain("133;A");
    }
  });
});
