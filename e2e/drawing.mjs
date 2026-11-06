// Phase 2 browser test: draw a flowchart by hand.
// Run with ./scripts/e2e.sh (which runs every e2e/*.mjs against one server).
// Usage: node drawing.mjs <login-link> <token>
import { chromium } from "playwright";

const [link, token] = process.argv.slice(2);
const BASE = "http://localhost:7070";
const OUT = new URL("./screenshots/", import.meta.url).pathname;
const exe = process.env.CHROMIUM_PATH || undefined;

const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  (${detail})` : ""}`);
};
const api = async (path) =>
  (await fetch(`${BASE}/api${path}`, { headers: { Authorization: `Bearer ${token}` } })).json();

const browser = await chromium.launch(exe ? { executablePath: exe } : {});
const ctx = await browser.newContext({ viewport: { width: 1400, height: 900 } });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
// Failed requests show up as console errors without a URL; record them properly.
page.on("console", (m) => m.type() === "error" && !m.text().startsWith("Failed to load resource") && errors.push(m.text()));
page.on("response", async (r) => {
  if (r.status() >= 400) errors.push(`${r.status()} ${r.request().method()} ${new URL(r.url()).pathname}: ${await r.text().catch(() => "")}`);
});

await page.goto(link);
await page.waitForURL(BASE + "/");
await page.getByLabel("New board title").fill("Checkout flow");
await page.getByRole("button", { name: "Create board" }).click();
await page.waitForURL(/\/boards\//);
const boardId = page.url().split("/").pop();
const mat = page.locator(".mat");
await mat.waitFor();
const box = await mat.boundingBox();
// An empty board opens at zoom 100% with canvas (0,0) at (80,80) on the mat.
const at = (x, y) => ({ x: box.x + 80 + x, y: box.y + 80 + y });
const board = () => api(`/boards/${boardId}`);
const settle = () => page.waitForTimeout(700);
const byText = (items, text) => items.find((i) => i.content?.text === text);
// New items get their editor after a server round trip; type once it has focus.
const typeNew = async (text) => {
  await page.locator(".text-editor:focus, .frame-title-input:focus").waitFor();
  await page.keyboard.type(text);
};

// ---------- Shapes and quick flow ----------

await page.keyboard.press("r");
let p = at(300, 100);
await page.mouse.click(p.x, p.y);
await typeNew("Basket");
await page.keyboard.press("Escape");
await settle();
let b = await board();
const basket = byText(b.items, "Basket");
check("R then click places a process step and opens its text", basket?.content.shape === "rect", JSON.stringify(basket?.content));

// Quick flow: the right-hand + handle adds a connected step.
await page.locator(".shape", { hasText: "Basket" }).click();
await page.getByRole("button", { name: "Add a connected step to the right" }).click();
await typeNew("Logged in?");
await page.keyboard.press("Escape");
await settle();
b = await board();
const auth = byText(b.items, "Logged in?");
const firstLink = b.connectors.find((c) => c.from_item === basket?.id && c.to_item === auth?.id);
check("a + handle adds a connected step beside it", !!auth && auth.x > basket.x + basket.w, `x ${auth?.x}`);
check("…joined by a connector from the right side to the left", firstLink?.from_anchor === "right" && firstLink?.to_anchor === "left");

// Make it a decision, then Tab adds the next step below.
await page.locator(".shape", { hasText: "Logged in?" }).click();
await page.getByLabel("Shape").selectOption("diamond");
await settle();
await page.keyboard.press("Tab");
await typeNew("Payment");
await page.keyboard.press("Escape");
await settle();
b = await board();
const pay = byText(b.items, "Payment");
check("changing the shape type keeps the step", byText(b.items, "Logged in?")?.content.shape === "diamond");
check("Tab adds the next step below", !!pay && pay.y > auth.y + auth.h && pay.content.shape === "rect", `shape ${pay?.content.shape}`);

// Drag from the left handle of Payment and drop on empty canvas.
await page.locator(".shape", { hasText: "Payment" }).click();
const left = await page.getByRole("button", { name: "Add a connected step to the left" }).boundingBox();
await page.mouse.move(left.x + 10, left.y + 10);
await page.mouse.down();
const drop = at(200, 420);
await page.mouse.move(drop.x, drop.y, { steps: 10 });
await page.mouse.up();
await typeNew("Log in");
await page.keyboard.press("Escape");
await settle();
b = await board();
const login = byText(b.items, "Log in");
check("dragging a handle to empty canvas adds a step there", !!login && Math.abs(login.x + login.w / 2 - 200) < 2, `centre x ${login && login.x + login.w / 2}`);
check("four steps and three connectors so far", b.items.length === 4 && b.connectors.length === 3, `${b.items.length} items, ${b.connectors.length} connectors`);

// Connector tool: drag from Log in to Payment.
await page.keyboard.press("c");
const lb = await page.locator(".shape", { hasText: "Log in" }).boundingBox();
const pb = await page.locator(".shape", { hasText: "Payment" }).boundingBox();
await page.mouse.move(lb.x + lb.width / 2, lb.y + lb.height / 2);
await page.mouse.down();
await page.mouse.move(pb.x + pb.width / 2, pb.y + pb.height / 2, { steps: 10 });
await page.mouse.up();
await settle();
b = await board();
check("the connector tool joins two items", b.connectors.length === 4 && b.connectors.some((c) => c.from_item === login.id));
check("…and drops back to the select tool", await page.getByRole("button", { name: "Select" }).getAttribute("aria-pressed") === "true");

await page.screenshot({ path: `${OUT}10-flow.png` });

// ---------- Connector label and style ----------

const conn = page.locator(".connector").first();
const path = await conn.locator(".connector-hit").boundingBox();
await page.mouse.click(path.x + path.width / 2, path.y + path.height / 2);
const bar = page.getByRole("toolbar", { name: "Connector style" });
await bar.waitFor();
await bar.getByLabel("Line").selectOption("dashed");
await page.locator(".connector-label").first().dblclick();
await page.keyboard.type("yes");
await page.keyboard.press("Enter");
await settle();
b = await board();
check("a connector's line style and label can be changed", b.connectors.some((c) => c.style?.line === "dashed" && c.label === "yes"), JSON.stringify(b.connectors.map((c) => [c.label, c.style])));

// ---------- Marquee, colour, frame ----------

await page.keyboard.press("Escape");
const from = at(100, 0);
const to = at(1000, 700);
await page.mouse.move(from.x, from.y);
await page.mouse.down();
await page.mouse.move(to.x, to.y, { steps: 10 });
await page.mouse.up();
check("dragging on empty canvas selects what the box encloses", (await page.locator(".item.is-selected").count()) === 4);
await page.getByRole("toolbar", { name: "Style" }).getByRole("button", { name: "blue" }).click();
await settle();
b = await board();
check("one swatch click colours the whole selection", b.items.every((i) => i.style?.color === "blue"));

await page.keyboard.press("f");
await typeNew("Checkout");
await page.keyboard.press("Enter");
await settle();
b = await board();
const frame = b.items.find((i) => i.kind === "frame");
const members = b.items.filter((i) => i.frame_id === frame?.id);
check("F frames the selection, and the frame holds its contents", frame?.title === "Checkout" && members.length === 4, `${members.length} members`);

// Drag the frame by its tab: everything inside moves.
const before = Object.fromEntries(b.items.map((i) => [i.id, [i.x, i.y]]));
const tab = await page.locator(".frame-tab").boundingBox();
await page.keyboard.down("Alt");
await page.mouse.move(tab.x + 20, tab.y + 8);
await page.mouse.down();
await page.mouse.move(tab.x + 120, tab.y + 58, { steps: 8 });
await page.mouse.up();
await page.keyboard.up("Alt");
await settle();
b = await board();
const movedAll = b.items.every((i) => i.x === before[i.id][0] + 100 && i.y === before[i.id][1] + 50);
check("dragging a frame moves everything in it", movedAll);

// Undo and redo the frame move.
await page.keyboard.press("ControlOrMeta+z");
await settle();
b = await board();
check("undo puts the frame and its contents back", b.items.every((i) => i.x === before[i.id][0] && i.y === before[i.id][1]));
await page.keyboard.press("ControlOrMeta+Shift+z");
await settle();
b = await board();
check("redo moves them again", b.items.every((i) => i.x === before[i.id][0] + 100));

// ---------- Delete, undo, copy, paste, duplicate ----------

await page.locator(".shape", { hasText: "Payment" }).click();
await page.keyboard.press("Delete");
await settle();
b = await board();
// Payment had three connectors (from Logged in?, to and from Log in); only Basket → Logged in? is left.
check("deleting a step hides its connectors", !byText(b.items, "Payment") && b.connectors.length === 1, `${b.connectors.length} connectors`);
await page.keyboard.press("ControlOrMeta+z");
await settle();
b = await board();
check("undo brings the step and its connectors back", !!byText(b.items, "Payment") && b.connectors.length === 4);

await page.locator(".shape", { hasText: "Basket" }).click();
await page.keyboard.press("ControlOrMeta+d");
await settle();
b = await board();
const copies = b.items.filter((i) => i.content?.text === "Basket");
check("Cmd+D duplicates", copies.length === 2 && copies.some((c) => c.x === copies[0].x + 20 || c.x === copies[1].x + 20));

await page.locator(".shape", { hasText: "Log in" }).click({ modifiers: ["Shift"] });
await page.locator(".shape", { hasText: "Payment" }).click({ modifiers: ["Shift"] });
await page.keyboard.press("Escape");
await page.locator(".shape", { hasText: "Log in" }).click();
await page.locator(".shape", { hasText: "Payment" }).click({ modifiers: ["Shift"] });
await page.keyboard.press("ControlOrMeta+c");
await page.keyboard.press("ControlOrMeta+v");
await settle();
b = await board();
const pastedLinks = b.connectors.filter((c) => {
  const f = b.items.find((i) => i.id === c.from_item);
  const t = b.items.find((i) => i.id === c.to_item);
  return f?.content.text === "Log in" && t?.content.text === "Payment";
});
check("copy and paste brings the connector between copied items", pastedLinks.length === 2, `${pastedLinks.length} Log in → Payment connectors`);

// ---------- Arrange and lock ----------

await page.keyboard.press("Escape");
// Two "Payment" shapes overlap after the paste; use the original by id.
await page.locator(".shape", { hasText: "Logged in?" }).click();
await page.locator(`[data-item="${pay.id}"]`).click({ position: { x: 6, y: 6 }, modifiers: ["Shift"] });
await page.getByRole("toolbar", { name: "Style" }).getByLabel("Arrange").selectOption("left");
await settle();
b = await board();
const lx = byText(b.items, "Logged in?").x;
check("Align left lines up the selection", b.items.filter((i) => i.content?.text === "Payment").some((i) => i.x === lx));

await page.locator(".shape", { hasText: "Logged in?" }).click();
await page.keyboard.press("ControlOrMeta+l");
await settle();
const lockedBefore = byText((await board()).items, "Logged in?");
const lk = await page.locator(".shape", { hasText: "Logged in?" }).boundingBox();
await page.mouse.move(lk.x + 20, lk.y + 20);
await page.mouse.down();
await page.mouse.move(lk.x + 200, lk.y + 20, { steps: 5 });
await page.mouse.up();
await settle();
const lockedAfter = byText((await board()).items, "Logged in?");
check("a locked item can't be dragged", lockedAfter.locked && lockedAfter.x === lockedBefore.x);

// ---------- Sticky and text ----------

await page.keyboard.press("Escape");
await page.keyboard.press("s");
p = at(1150, 100);
await page.mouse.click(p.x, p.y);
await typeNew("Guest checkout?");
await page.keyboard.press("Escape");
await page.keyboard.press("t");
p = at(300, -40);
await page.mouse.click(p.x, p.y);
await typeNew("Checkout: v1");
await page.keyboard.press("Escape");
await settle();
b = await board();
check("S adds a sticky note", b.items.some((i) => i.kind === "sticky" && i.content.text === "Guest checkout?"));

// A sticky takes the colour it's given (its yellow default mustn't win).
const sticky = page.locator(".sticky", { hasText: "Guest checkout?" });
const yellowBg = await sticky.evaluate((el) => getComputedStyle(el).backgroundColor);
await sticky.click();
await page.getByRole("toolbar", { name: "Style" }).getByRole("button", { name: "pink" }).click();
await settle();
const pinkBg = await sticky.evaluate((el) => getComputedStyle(el).backgroundColor);
check("recolouring a sticky changes its colour", yellowBg !== pinkBg, `${yellowBg} → ${pinkBg}`);

// The style bar stays clear of the toolbar even for items at the left edge.
await page.locator(".free-text", { hasText: "Checkout: v1" }).click();
const barBox = await page.getByRole("toolbar", { name: "Style" }).boundingBox();
const toolsBox = await page.getByRole("navigation", { name: "Tools" }).boundingBox();
check("the style bar never covers the toolbar", barBox.x >= toolsBox.x + toolsBox.width, `bar x ${Math.round(barBox.x)}, tools end ${Math.round(toolsBox.x + toolsBox.width)}`);
check("T adds free text", b.items.some((i) => i.kind === "text" && i.content.text === "Checkout: v1"));

await page.keyboard.press("Shift+Digit1");
await page.waitForTimeout(300);
await page.screenshot({ path: `${OUT}11-board-light.png` });
const dark = await browser.newContext({ viewport: { width: 1400, height: 900 }, colorScheme: "dark" });
await dark.addCookies(await ctx.cookies());
const dp = await dark.newPage();
await dp.goto(`${BASE}/boards/${boardId}`);
await dp.locator(".shape").first().waitFor();
await dp.waitForTimeout(300);
await dp.screenshot({ path: `${OUT}12-board-dark.png` });

check("no JavaScript errors", errors.length === 0, errors.join(" | "));
await browser.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
