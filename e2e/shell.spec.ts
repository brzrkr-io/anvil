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

test("the + menu opens with New… actions", async ({ page }) => {
  await page.locator('.newtab[title="New…"]').click();
  const menu = page.locator(".plusmenu");
  await expect(menu).toBeVisible();
  await expect(menu.getByText("New Terminal")).toBeVisible();
  await expect(menu.getByText("Web Preview")).toBeVisible();
});
