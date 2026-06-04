import { test, expect } from "@playwright/test";

// Smoke tests for the app shell (no Tauri backend in the browser).
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("anvil-onboarded", "1"));
  await page.goto("/");
  // Dismiss the one-time "What's New" modal if it shows — it overlays the shell
  // and blocks keyboard/click interaction.
  await page.locator(".wn-go").click({ timeout: 2000 }).catch(() => {});
});

test("renders the app shell with the activity rail", async ({ page }) => {
  await expect(page.locator(".rail")).toBeVisible();
  await expect(page.locator(".rail .i")).not.toHaveCount(0);
});

test("command palette opens with ⌘K and filters", async ({ page }) => {
  await page.keyboard.press("Meta+k");
  const input = page.locator(".palette input");
  await expect(input).toBeVisible();
  await input.fill("terminal");
  await expect(page.locator(".palette .pi").first()).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(input).toBeHidden();
});

test("activity rail retargets the active pane's view (always-grid)", async ({ page }) => {
  // Always-grid shell (cda9d56): the content is permanently the PaneGrid; a rail
  // icon click sets the ACTIVE pane's view instead of swapping a full-screen view.
  const scmIcon = page.locator('.rail .i[title^="Source Control"]');
  await scmIcon.click();
  // The clicked rail icon reflects the active pane's view by going active…
  await expect(scmIcon).toHaveClass(/\bon\b/);
  // …and the Source Control surface renders inside the grid leaf, not a pane-head view.
  const leaf = page.locator("[data-leaf-id]").first();
  await expect(leaf).toBeVisible();
  await expect(leaf.locator(".scm")).toBeVisible();
});

test("rail icons retarget the same pane rather than stacking views", async ({ page }) => {
  // The refactor (cda9d56) made rail buttons drive the active pane: clicking a
  // second rail view REPLACES the first in the same leaf — it does not open a new
  // pane or a separate top-bar tab. Exactly one leaf, showing the last view picked.
  await page.locator('.rail .i[title^="Source Control"]').click();
  await expect(page.locator("[data-leaf-id]").first().locator(".scm")).toBeVisible();

  await page.locator('.rail .i[title^="Search"]').click();
  const leaves = page.locator("[data-leaf-id]");
  await expect(leaves).toHaveCount(1); // retargeted, not stacked into a new pane
  const leaf = leaves.first();
  await expect(leaf.locator(".sp")).toBeVisible(); // Search panel now fills the pane
  await expect(leaf.locator(".scm")).toHaveCount(0); // the previous view was replaced
});

test("the + menu opens with New… actions", async ({ page }) => {
  await page.locator('.newtab[title="New…"]').click();
  const menu = page.locator(".plusmenu");
  await expect(menu).toBeVisible();
  await expect(menu.getByText("New Terminal")).toBeVisible();
  await expect(menu.getByText("Web Preview")).toBeVisible();
});

test("the Basin brand mark anchors the rail", async ({ page }) => {
  await expect(page.locator(".rail .brandmark")).toBeVisible();
});

test("command palette surfaces ops commands", async ({ page }) => {
  await page.keyboard.press("Meta+k");
  const input = page.locator(".palette input");
  await input.fill("Run Snippet");
  await expect(page.locator(".palette .pi").filter({ hasText: "Run Snippet" }).first()).toBeVisible();
  await input.fill("Secrets");
  await expect(page.locator(".palette .pi").filter({ hasText: "Secrets" }).first()).toBeVisible();
  await page.keyboard.press("Escape");
});

test("the snippets palette lists default DevOps commands", async ({ page }) => {
  await page.keyboard.press("Meta+k");
  await page.locator(".palette input").fill("Run Snippet");
  await page.locator(".palette .pi").filter({ hasText: "Run Snippet" }).first().click();
  await page.locator(".palette input").fill(""); // sub-palette keeps the old filter
  await expect(page.locator(".palette .pi").filter({ hasText: "kubectl" }).first()).toBeVisible();
  await page.keyboard.press("Escape");
});
