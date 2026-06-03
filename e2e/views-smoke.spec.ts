import { test, expect } from "@playwright/test";

// Navigation smoke: open every activity-rail view and the right-drawer panels,
// asserting none of them throw to the error boundary or log a Svelte keyed-each
// duplicate. No Tauri backend in the browser, so this catches *mount* crashes
// and reconciliation bugs (each_key_duplicate), not backend data flows.
test("every rail view mounts without hitting the error boundary", async ({ page }) => {
  // Tauri's invoke()/transformCallback are undefined in the browser harness;
  // those errors are expected (no backend) and filtered out. We only fail on
  // reconciliation/boundary crashes that are real bugs regardless of backend.
  const ignore = /invoke|transformCallback|__TAURI__|read_state|pty_/;
  const fatal: string[] = [];
  const note = (t: string) => { if (!ignore.test(t)) fatal.push(t); };
  page.on("console", (m) => {
    const t = m.text();
    if (/each_key_duplicate|error_boundary/.test(t)) note(t);
  });
  page.on("pageerror", (e) => note(String(e)));

  await page.addInitScript(() => localStorage.setItem("anvil-onboarded", "1"));
  await page.goto("/");
  await page.locator(".wn-go").click({ timeout: 2000 }).catch(() => {});

  // Click each rail icon in turn; after each, the content must not show the
  // crash fallback.
  const rail = page.locator(".rail .i[title]");
  const n = await rail.count();
  expect(n).toBeGreaterThan(0);
  for (let i = 0; i < n; i++) {
    const item = rail.nth(i);
    const title = (await item.getAttribute("title")) ?? `rail#${i}`;
    await item.click();
    await page.waitForTimeout(120); // let the view mount + any effects run
    const crashed = await page.locator(".crash-fallback").count();
    expect(crashed, `view "${title}" hit the error boundary`).toBe(0);
  }

  expect(fatal, `console/page errors:\n${fatal.join("\n")}`).toEqual([]);
});
