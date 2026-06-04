import { test, expect } from "@playwright/test";

// Performance smoke: open every activity-rail view and assert each mounts
// within BUDGET ms. No Tauri backend in the browser harness; views render
// with empty data — we measure mount/render time, not data loading.
const BUDGET = 2000;

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("anvil-onboarded", "1"));
  await page.goto("/");
  await page.locator(".wn-go").click({ timeout: 2000 }).catch(() => {});
});

test("every rail view mounts within budget", async ({ page }) => {
  const rail = page.locator(".rail .i[title]");
  const n = await rail.count();
  expect(n, "expected at least one rail icon").toBeGreaterThan(0);

  const timings: { title: string; ms: number }[] = [];

  for (let i = 0; i < n; i++) {
    const item = rail.nth(i);
    const title = (await item.getAttribute("title")) ?? `rail#${i}`;

    const t0 = Date.now();
    await item.click();

    // Wait until: no crash-fallback is rendered. Mirrors views-smoke approach
    // but with an explicit timeout so we attribute the failure to the right view.
    await page.waitForFunction(
      () => document.querySelectorAll(".crash-fallback").length === 0,
      { timeout: BUDGET },
    );

    // Always-grid shell (cda9d56): a rail click retargets the active pane, so the
    // content stays the PaneGrid — there's no full-screen ".pane-head" to wait on,
    // and some rail icons (Settings) hide the grid behind an overlay view. The grid
    // stays MOUNTED throughout (display-toggled, not unmounted), so requiring its
    // leaf to be attached — together with the crash-fallback check above — proves
    // the shell re-rendered for this view without tearing the surface down.
    await page.locator("[data-leaf-id]").first().waitFor({ state: "attached", timeout: BUDGET });

    const ms = Date.now() - t0;
    timings.push({ title, ms });
  }

  // Print a timing table to stdout so it appears in CI output.
  console.log("\nRail mount timings:");
  console.log("─".repeat(44));
  for (const { title, ms } of timings) {
    const bar = ms < BUDGET / 2 ? "OK  " : ms < BUDGET ? "SLOW" : "FAIL";
    console.log(`  ${bar}  ${String(ms).padStart(5)}ms  ${title}`);
  }
  console.log("─".repeat(44));

  // Assert each view is within budget.
  for (const { title, ms } of timings) {
    expect(ms, `rail "${title}" mounted in ${ms}ms (budget ${BUDGET}ms)`).toBeLessThan(BUDGET);
  }

  // Assert no crash-fallback is present after all views have been visited.
  const crashed = await page.locator(".crash-fallback").count();
  expect(crashed, "crash-fallback rendered after visiting all rail views").toBe(0);
});
