// Drives Board's UI in a real browser against a running server.
// Run it with ./scripts/e2e.sh, which starts a throwaway server first.
// Usage: node run.mjs <login-link> <token>
import { chromium } from "playwright";

const [link, token] = process.argv.slice(2);
const BASE = "http://localhost:7070";
const OUT = new URL("./screenshots/", import.meta.url).pathname;
// Use a specific Chromium if given (handy when the cached browser doesn't
// match this Playwright version); otherwise Playwright's own.
const exe = process.env.CHROMIUM_PATH || undefined;

const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  (${detail})` : ""}`);
};
const apiGet = async (path) =>
  (await fetch(`${BASE}/api${path}`, { headers: { Authorization: `Bearer ${token}` } })).json();

const browser = await chromium.launch(exe ? { executablePath: exe } : {});
const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
// The logged-out check at the start makes one expected 401.
page.on("console", (m) => m.type() === "error" && !m.text().includes("401") && errors.push(m.text()));

// Logged out first.
await page.goto(BASE);
check("logged-out page explains how to log in", await page.getByText("board login").isVisible());

// Log in with the one-time link.
await page.goto(link);
await page.waitForURL(BASE + "/");
await page.getByRole("heading", { name: "Boards" }).waitFor();
check("login link lands on the board list", true);
await page.screenshot({ path: `${OUT}01-empty-list.png` });

// Create a board.
await page.getByLabel("New board title").fill("Acme rebuild");
await page.getByRole("button", { name: "Create board" }).click();
await page.waitForURL(/\/boards\//);
const boardId = page.url().split("/").pop();
await page.getByText("Double-click anywhere").waitFor();
check("new board opens on an empty mat with a hint", true);

// Add a note by double-clicking the mat.
const mat = page.locator(".mat");
await mat.dblclick({ position: { x: 300, y: 250 } });
await page.getByRole("menuitem", { name: "Note", exact: true }).click();
const note = page.locator(".card").filter({ has: page.locator(".note-view, .note-editor") });
await note.waitFor();
await note.locator(".note-view").dblclick();
await page.keyboard.type("# Kickoff\n\nGoals for **phase one**.\n\n- Sitemap\n- Data model\n<script>alert(1)</script>");
await page.keyboard.press("Escape");
await page.waitForTimeout(1200);
let full = await apiGet(`/boards/${boardId}`);
const noteItem = full.items.find((i) => i.card_type === "note");
check("note text saved to the server", noteItem?.content.markdown.includes("phase one"), `version ${noteItem?.version}`);
check("markdown renders as HTML", (await note.locator(".markdown strong").innerText()) === "phase one");
check("raw HTML is shown as text, not run", (await note.locator(".markdown").innerText()).includes("<script>"));

// Rename the note.
await note.locator(".card-title").dblclick();
await page.keyboard.press("ControlOrMeta+a");
await page.keyboard.type("Kickoff notes");
await page.keyboard.press("Enter");
await page.waitForTimeout(1000);
full = await apiGet(`/boards/${boardId}`);
check("card title saved", full.items.find((i) => i.id === noteItem.id)?.title === "Kickoff notes");

// Add a checklist and fill it in.
await mat.dblclick({ position: { x: 800, y: 300 } });
await page.getByRole("menuitem", { name: "Checklist", exact: true }).click();
const list = page.locator(".card").filter({ has: page.locator(".checklist") });
await list.waitFor();
await list.locator(".checklist-text").first().fill("Register domain");
await list.locator(".checklist-text").first().press("Enter");
await page.keyboard.type("Set up hosting");
await list.locator('input[type="checkbox"]').first().check();
await page.waitForTimeout(1200);
full = await apiGet(`/boards/${boardId}`);
const listItem = full.items.find((i) => i.card_type === "checklist");
check(
  "checklist items and ticks saved",
  listItem?.content.items.length === 2 &&
    listItem.content.items[0].text === "Register domain" && listItem.content.items[0].done === true &&
    listItem.content.items[1].text === "Set up hosting" && listItem.content.items[1].done === false,
  JSON.stringify(listItem?.content.items),
);
check("checklist shows the count", (await list.locator(".checklist-count").innerText()) === "1 of 2 done");

// Drag the note by its header.
const before = full.items.find((i) => i.id === noteItem.id);
const header = note.locator(".card-header");
const hb = await header.boundingBox();
// Hold Alt: drags snap to a 10-unit grid otherwise.
await page.keyboard.down("Alt");
await page.mouse.move(hb.x + 40, hb.y + 10);
await page.mouse.down();
await page.mouse.move(hb.x + 140, hb.y + 70, { steps: 8 });
await page.mouse.up();
await page.keyboard.up("Alt");
await page.waitForTimeout(600);
full = await apiGet(`/boards/${boardId}`);
const moved = full.items.find((i) => i.id === noteItem.id);
const zoom = parseInt(await page.locator(".zoom-level").innerText()) / 100;
const dx = Math.round((moved.x - before.x) * zoom);
const dy = Math.round((moved.y - before.y) * zoom);
check("drag moves the card and saves its position", Math.abs(dx - 100) <= 2 && Math.abs(dy - 60) <= 2, `moved ${dx},${dy} screen px at ${zoom * 100}%`);

// Resize from the corner.
const rb = await note.locator(".card-resize").boundingBox();
await page.mouse.move(rb.x + 8, rb.y + 8);
await page.mouse.down();
await page.mouse.move(rb.x + 88, rb.y + 48, { steps: 6 });
await page.mouse.up();
await page.waitForTimeout(600);
full = await apiGet(`/boards/${boardId}`);
const resized = full.items.find((i) => i.id === noteItem.id);
check("resize saves the new size", resized.w > moved.w && resized.h > moved.h, `${moved.w}×${moved.h} → ${resized.w}×${resized.h}`);

// Reload: positions persist.
await page.reload();
await page.locator(".card").first().waitFor();
check("cards come back after a reload", (await page.locator(".card").count()) === 2);

// Light-mode screenshot of the board.
await page.screenshot({ path: `${OUT}02-board-light.png` });

// Two tabs editing the same note: the second save conflicts.
const page2 = await ctx.newPage();
await page2.goto(`${BASE}/boards/${boardId}`);
await page2.locator(".card").first().waitFor();
const note1 = page.locator(".card").filter({ hasText: "Kickoff notes" });
const note2 = page2.locator(".card").filter({ hasText: "Kickoff notes" });
await note1.locator(".note-view").dblclick();
await page.keyboard.press("ControlOrMeta+End");
await page.keyboard.type("\nEdited in tab one.");
await page.keyboard.press("Escape");
await page.waitForTimeout(1200);
await note2.locator(".note-view").dblclick();
await page2.keyboard.press("ControlOrMeta+End");
await page2.keyboard.type("\nEdited in tab two.");
await page2.keyboard.press("Escape");
await note2.locator(".card-conflict").waitFor({ timeout: 5000 });
check("a stale edit in another tab shows the conflict banner", true);
await page2.screenshot({ path: `${OUT}03-conflict.png` });
await note2.getByRole("button", { name: "Keep mine" }).click();
await page2.waitForTimeout(1200);
full = await apiGet(`/boards/${boardId}`);
const kept = full.items.find((i) => i.id === noteItem.id);
check("Keep mine saves tab two's text", kept.content.markdown.includes("tab two") && !kept.content.markdown.includes("tab one"));
await page2.close();

// Delete with the keyboard, then restore from the trash.
await page.reload();
await page.locator(".card").first().waitFor();
await page.locator(".card").filter({ has: page.locator(".checklist") }).locator(".card-header").click();
await page.keyboard.press("Delete");
await page.waitForTimeout(500);
check("Delete key moves the selected card to the trash", (await page.locator(".card").count()) === 1);
await page.getByRole("button", { name: "Trash", exact: true }).click();
await page.getByRole("button", { name: "Restore" }).click();
await page.waitForTimeout(500);
check("Restore brings it back", (await page.locator(".card").count()) === 2);
await page.screenshot({ path: `${OUT}04-trash.png` });

// Zoom with Ctrl+wheel (a trackpad pinch).
const zBefore = await page.locator(".zoom-level").innerText();
await page.mouse.move(640, 400);
await page.keyboard.down("Control");
await page.mouse.wheel(0, -100);
await page.keyboard.up("Control");
const zAfter = await page.locator(".zoom-level").innerText();
check("one Ctrl+wheel notch zooms one step, not to the limit", zBefore !== zAfter && zAfter !== "400%", `${zBefore} → ${zAfter}`);

// Board list shows the board with its card count; dark mode screenshots.
await page.goto(BASE);
await page.getByText("Acme rebuild").waitFor();
check("board list shows the card count", await page.getByText("2 cards").isVisible());
await page.screenshot({ path: `${OUT}05-list-light.png` });
const dark = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: "dark" });
await dark.addCookies(await ctx.cookies());
const dp = await dark.newPage();
await dp.goto(`${BASE}/boards/${boardId}`);
await dp.locator(".card").first().waitFor();
await dp.screenshot({ path: `${OUT}06-board-dark.png` });

check("no JavaScript errors", errors.length === 0, errors.join(" | "));
await browser.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
