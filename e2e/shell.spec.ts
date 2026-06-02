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

test("activity rail switches the main view", async ({ page }) => {
  await page.locator('.rail .i[title^="Source Control"]').click();
  await expect(page.locator(".pane-head")).toContainText(/Source Control/i);
});

test("rail view opens as a closable tab (unified tab model)", async ({ page }) => {
  await page.locator('.rail .i[title^="Source Control"]').click();
  const tab = page.locator(".tabs .tab").filter({ hasText: "Source Control" });
  await expect(tab).toBeVisible();
  await tab.locator(".x").click();
  await expect(tab).toBeHidden();
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
