/**
 * RFC-091 — What’s this? inspect mode.
 *
 * Runs against public pages so auth is not required.
 */

import { test, expect } from "@playwright/test";

test("What’s this? toggle enters inspect mode and outlines help targets", async ({ page }) => {
  await page.goto("/pricing");
  await page.getByRole("button", { name: "What’s this?" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-help-mode", "");
  await expect(page.locator("[data-help]").first()).toBeVisible();
  await expect(page.getByText("click a highlighted control")).toBeVisible();
});

test("clicking a highlighted nav item opens the help card and does not navigate", async ({ page }) => {
  await page.goto("/pricing");
  await page.getByRole("button", { name: "What’s this?" }).click();

  await page.getByRole("link", { name: "Dashboard" }).click();
  await expect(page).toHaveURL(/\/pricing/);
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByRole("dialog").getByRole("heading", { name: "Dashboard" })).toBeVisible();
  await expect(page.getByRole("dialog").getByText(/Live ingestion health/)).toBeVisible();
});

test("clicking an unregistered area shows the miss toast", async ({ page }) => {
  await page.goto("/pricing");
  await page.getByRole("button", { name: "What’s this?" }).click();

  await page.locator("main").click({ position: { x: 8, y: 8 } });
  await expect(page.getByText("No description for this yet.")).toBeVisible();
});

test("Escape closes the card, then exits mode", async ({ page }) => {
  await page.goto("/pricing");
  await page.getByRole("button", { name: "What’s this?" }).click();
  await page.getByRole("link", { name: "Dashboard" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator("html")).toHaveAttribute("data-help-mode", "");

  await page.keyboard.press("Escape");
  await expect(page.locator("html")).not.toHaveAttribute("data-help-mode");
});

test("? toggles inspect mode when not typing", async ({ page }) => {
  await page.goto("/pricing");
  await page.locator("body").click();
  await page.keyboard.press("Shift+/");
  await expect(page.locator("html")).toHaveAttribute("data-help-mode", "");
  await page.keyboard.press("Shift+/");
  await expect(page.locator("html")).not.toHaveAttribute("data-help-mode");
});

test("hover still mounts a tooltip when inspect mode is off", async ({ page }) => {
  await page.goto("/pricing");
  await page.getByRole("link", { name: "Dashboard" }).hover();
  await page.waitForTimeout(400);
  const tooltip = page.locator("[data-radix-tooltip-content], [role='tooltip']");
  await expect(tooltip.first()).toBeVisible({ timeout: 2000 });
  await expect(tooltip.first()).toContainText(/ingestion health/i);
});
