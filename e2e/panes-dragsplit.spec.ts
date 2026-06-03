import { test, expect, type Page } from "@playwright/test";

// Pointer-based drag-to-split + the split-fill layout (no HTML5 drag-and-drop,
// which silently no-ops in the app's WebView). Runs against the static build —
// no Tauri backend — so this drives pane tabs (present after a split) rather
// than file tabs (which need a backend to open). The top-strip file/view tab
// path shares the same startTabDrag controller; its drop math is unit-tested in
// src/lib/tabdrag.test.ts.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("anvil-onboarded", "1"));
  await page.goto("/");
  await page.locator(".wn-go").click({ timeout: 2000 }).catch(() => {});
});

async function pressPastThreshold(page: Page, sx: number, sy: number) {
  await page.mouse.move(sx, sy);
  await page.mouse.down();
  await page.mouse.move(sx + 8, sy + 8); // > 5px → commits the drag
}

test("a row split fills its cell edge-to-edge (no floating box)", async ({ page }) => {
  await page.keyboard.press("Meta+\\"); // split right
  const leaves = page.locator(".grid-fill .leaf");
  await expect(leaves).toHaveCount(2);
  const grid = (await page.locator(".grid-fill").boundingBox())!;
  const a = (await leaves.nth(0).boundingBox())!;
  const b = (await leaves.nth(1).boundingBox())!;
  // Full height, equal half-widths, anchored at the grid's top-left.
  expect(a.height).toBeGreaterThan(grid.height - 2);
  expect(b.height).toBeGreaterThan(grid.height - 2);
  expect(Math.abs(a.width - b.width)).toBeLessThan(4);
  expect(a.y).toBeLessThan(grid.y + 2);
  expect(a.x).toBeLessThan(grid.x + 2);
});

test("a column split fills its cell", async ({ page }) => {
  await page.keyboard.press("Meta+Shift+\\"); // split down
  const leaves = page.locator(".grid-fill .leaf");
  await expect(leaves).toHaveCount(2);
  const grid = (await page.locator(".grid-fill").boundingBox())!;
  const a = (await leaves.nth(0).boundingBox())!;
  const b = (await leaves.nth(1).boundingBox())!;
  expect(a.width).toBeGreaterThan(grid.width - 2);
  expect(b.width).toBeGreaterThan(grid.width - 2);
  expect(Math.abs(a.height - b.height)).toBeLessThan(4);
});

test("dragging a pane tab to a pane's edge splits that pane", async ({ page }) => {
  await page.keyboard.press("Meta+\\");
  await expect(page.locator(".grid-fill .leaf")).toHaveCount(2);
  // Second tab in pane 0 so moving one tab out won't collapse the pane.
  await page.locator(".grid-fill .leaf").nth(0).locator(".phead .ptadd").click();
  await expect(page.locator(".grid-fill .leaf").nth(0).locator(".ptab")).toHaveCount(2);

  const sb = (await page.locator(".grid-fill .leaf").nth(0).locator(".phead .ptab").first().boundingBox())!;
  const tb = (await page.locator(".grid-fill .leaf").nth(1).boundingBox())!;

  await pressPastThreshold(page, sb.x + sb.width / 2, sb.y + sb.height / 2);
  await page.mouse.move(tb.x + tb.width / 2, tb.y + tb.height / 2);
  await expect(page.locator(".dragghost")).toBeVisible();          // ghost follows cursor
  await page.mouse.move(tb.x + tb.width - 10, tb.y + tb.height / 2);
  await expect(page.locator(".dropzone.right")).toBeVisible();      // edge highlight on target
  await page.mouse.up();

  await expect(page.locator(".grid-fill .leaf")).toHaveCount(3);    // target split → +1 pane
  await expect(page.locator(".dragghost")).toHaveCount(0);          // ghost cleared on release
});

test("dropping a pane tab in a pane's center moves it as a tab", async ({ page }) => {
  await page.keyboard.press("Meta+\\");
  await expect(page.locator(".grid-fill .leaf")).toHaveCount(2);
  await page.locator(".grid-fill .leaf").nth(0).locator(".phead .ptadd").click();
  await expect(page.locator(".grid-fill .leaf").nth(0).locator(".ptab")).toHaveCount(2);

  const sb = (await page.locator(".grid-fill .leaf").nth(0).locator(".phead .ptab").first().boundingBox())!;
  const tb = (await page.locator(".grid-fill .leaf").nth(1).boundingBox())!;

  await pressPastThreshold(page, sb.x + sb.width / 2, sb.y + sb.height / 2);
  await page.mouse.move(tb.x + tb.width / 2, tb.y + tb.height / 2);
  await expect(page.locator(".dropzone.center")).toBeVisible();
  await page.mouse.up();

  await expect(page.locator(".grid-fill .leaf")).toHaveCount(2);    // no new pane
  await expect(page.locator(".grid-fill .leaf").nth(0).locator(".ptab")).toHaveCount(1); // source lost one
  await expect(page.locator(".grid-fill .leaf").nth(1).locator(".ptab")).toHaveCount(2); // target gained one
});

test("a sub-threshold press selects the tab (no drag ghost, no split)", async ({ page }) => {
  await page.keyboard.press("Meta+\\");
  await expect(page.locator(".grid-fill .leaf")).toHaveCount(2);
  const b = (await page.locator(".grid-fill .leaf").nth(0).locator(".phead .ptab").first().boundingBox())!;
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
  await page.mouse.down();
  await page.mouse.move(b.x + b.width / 2 + 2, b.y + b.height / 2); // < 5px
  await page.mouse.up();
  await expect(page.locator(".dragghost")).toHaveCount(0);
  await expect(page.locator(".grid-fill .leaf")).toHaveCount(2);
});
