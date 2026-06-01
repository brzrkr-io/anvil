import { test, expect } from "@playwright/test";

// #80 Golden screenshot tests for the app chrome. Snapshots are stored next to
// this spec; update them with `npx playwright test --update-snapshots`.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("anvil-onboarded", "1"));
  await page.goto("/");
  await page.locator(".rail").waitFor();
});

test("activity rail chrome matches golden", async ({ page }) => {
  // Snapshot the stable left rail (deterministic icons) rather than the whole
  // shell, whose main view carries dynamic content.
  await expect(page.locator(".rail")).toHaveScreenshot("rail.png", { animations: "disabled", maxDiffPixelRatio: 0.05 });
});

test("command palette matches golden", async ({ page }) => {
  await page.keyboard.press("Meta+k");
  await page.locator(".palette input").waitFor();
  await expect(page.locator(".palette")).toHaveScreenshot("palette.png", { animations: "disabled", maxDiffPixelRatio: 0.05 });
});
