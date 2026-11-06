// Phase 3 browser test: live updates and history.
// Run with ./scripts/e2e.sh. Usage: node live.mjs <login-link> <admin-token>
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
const call = async (method, path, body) => {
  const res = await fetch(`${BASE}/api${path}`, {
    method,
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  return res.status === 204 ? null : res.json();
};

const browser = await chromium.launch(exe ? { executablePath: exe } : {});
const ctx = await browser.newContext({ viewport: { width: 1400, height: 900 } });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
page.on("response", async (r) => {
  if (r.status() >= 400) errors.push(`${r.status()} ${new URL(r.url()).pathname}`);
});

await page.goto(link);
await page.waitForURL(BASE + "/");
const board = await call("POST", "/boards", { title: "Live board" });
await page.goto(`${BASE}/boards/${board.id}`);
await page.locator(".mat").waitFor();
await page.waitForTimeout(500); // let the event stream connect

// ---------- Changes from elsewhere appear without a reload ----------

const flow = await call("POST", `/boards/${board.id}/flows`, {
  mermaid: "flowchart LR\n  a(Enquiry) --> b{Quote ok?}\n  b -->|yes| c[Book]\n  b -->|no| d[Follow up]",
  frame_title: "Sales",
});
await page.locator(".shape", { hasText: "Quote ok?" }).waitFor({ timeout: 3000 });
check("a flow drawn through the API appears without a reload", (await page.locator(".shape").count()) === 4);
check("…including its frame and connectors", (await page.locator(".frame").count()) === 1 && (await page.locator(".connector").count()) === 3);
check("…highlighted as someone else's change", (await page.locator(".item.is-fresh").count()) >= 4);
await page.screenshot({ path: `${OUT}20-live-flow.png` });
await page.waitForTimeout(2800);
check("the highlight fades", (await page.locator(".item.is-fresh").count()) === 0);

// Edits and deletes from elsewhere.
const book = flow.items.c;
const bookItem = await call("GET", `/items/${book}`);
await call("PATCH", `/items/${book}`, { version: bookItem.version, content: { shape: "rect", text: "Book a call" } });
await page.locator(".shape", { hasText: "Book a call" }).waitFor({ timeout: 3000 });
check("an edit from elsewhere updates the shape", true);
await call("DELETE", `/items/${flow.items.d}`);
await page.waitForFunction(() => ![...document.querySelectorAll(".shape")].some((s) => s.textContent.includes("Follow up")), null, { timeout: 3000 });
check("a delete from elsewhere removes it", (await page.locator(".connector").count()) === 2);

// ---------- Two tabs ----------

const other = await ctx.newPage();
await other.goto(`${BASE}/boards/${board.id}`);
await other.locator(".shape").first().waitFor();
await other.waitForTimeout(500);
await other.keyboard.press("s");
const mat = await other.locator(".mat").boundingBox();
await other.mouse.click(mat.x + 200, mat.y + 600);
await other.locator(".text-editor:focus").waitFor();
await other.keyboard.type("From tab two");
await other.keyboard.press("Escape");
await page.locator(".sticky", { hasText: "From tab two" }).waitFor({ timeout: 4000 });
check("a sticky made in one tab appears in the other", true);
check("…and the tab that made it doesn't flash it as someone else's", (await other.locator(".item.is-fresh").count()) === 0);
await other.close();

// ---------- History: undo what "Claude" did ----------

const note = await call("POST", `/boards/${board.id}/items`, {
  kind: "card", card_type: "note", title: "Scope", content: { markdown: "Five pages." }, x: 800, y: 500,
});
const card = page.locator(".card", { hasText: "Scope" });
await card.waitFor();
// The user edits it (a second version, by a different person from the script that made it).
await card.locator(".note-view").dblclick();
await page.keyboard.press("ControlOrMeta+End");
await page.keyboard.type(" Plus a shop.");
await page.keyboard.press("Escape");
await page.waitForTimeout(1200);
await card.locator(".card-header").click();
await page.getByRole("toolbar", { name: "Style" }).getByRole("button", { name: "History" }).click();
const panel = page.getByRole("complementary", { name: "History" });
await panel.locator("li").nth(1).waitFor();
check("History lists who made each version", (await panel.locator("li").count()) === 2 && (await panel.locator("li").first().innerText()).startsWith("You"));
await page.screenshot({ path: `${OUT}21-history.png` });
await panel.getByRole("button", { name: "Restore" }).click();
await page.waitForTimeout(600);
const restored = await call("GET", `/items/${note.id}`);
check("Restore puts the earlier version back", restored.content.markdown === "Five pages.", restored.content.markdown);
check("…and the card shows it", (await card.locator(".markdown").innerText()).trim() === "Five pages.");

check("no errors", errors.length === 0, errors.join(" | "));
await browser.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
