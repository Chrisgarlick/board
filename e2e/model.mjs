// Phase 6 browser test: plan a feature on a card, then read the board model
// the way Design will: over HTTP with ETags, and with `board pull`.
// Run with ./scripts/e2e.sh. Usage: node model.mjs <login-link> <admin-token>
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromium } from "playwright";

const [link, token] = process.argv.slice(2);
const BASE = "http://localhost:7070";
const OUT = new URL("./screenshots/", import.meta.url).pathname;
const BIN = new URL("../target/debug/board", import.meta.url).pathname;
const exe = process.env.CHROMIUM_PATH || undefined;

const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  (${detail})` : ""}`);
};
const call = async (method, path, body) =>
  (
    await fetch(`${BASE}/api${path}`, {
      method,
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: body ? JSON.stringify(body) : undefined,
    })
  ).json();
const settle = () => new Promise((r) => setTimeout(r, 900));

// A board with a table, a schema card and a sitemap, made through the API.
const board = await call("POST", "/boards", { title: "Florist site" });
const orders = (
  await call("POST", `/boards/${board.id}/tables`, {
    name: "orders",
    definition: { columns: [{ name: "id", type: "bigint", primary: true }, { name: "status", type: "enum", values: ["new", "sent"] }] },
  })
).table;
const schemaCard = await call("POST", `/boards/${board.id}/items`, {
  kind: "card", card_type: "schema", title: "Orders data", content: { tables: ["orders"] }, x: 900, y: 600,
});
await call("POST", `/boards/${board.id}/items`, {
  kind: "card", card_type: "sitemap", title: "Sitemap", x: 900, y: 0,
  content: { root: { title: "Home", slug: "/", children: [{ title: "Checkout", slug: "/checkout" }] } },
});

const browser = await chromium.launch(exe ? { executablePath: exe } : {});
const page = await (await browser.newContext({ viewport: { width: 1500, height: 950 } })).newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
page.on("response", (r) => r.status() >= 500 && errors.push(`${r.status()} ${new URL(r.url()).pathname}`));

await page.goto(link);
await page.waitForURL(BASE + "/");
await page.goto(`${BASE}/boards/${board.id}`);
await page.locator(".card-schema").waitFor();

// ---------- A feature card ----------
await page.mouse.dblclick(200, 200);
await page.getByRole("menuitem", { name: "Feature" }).click();
const feature = page.locator(".card-feature");
await feature.waitFor();
const featureId = await feature.getAttribute("data-item");
await feature.getByRole("radio", { name: "Planned" }).click();
await feature.locator(".feature-summary").dblclick();
await feature.getByLabel("Summary (markdown)").fill("Customers pay by card at **checkout**.");
await feature.getByLabel("Summary (markdown)").press("Escape");
await feature.getByRole("button", { name: "Add story" }).click();
await page.keyboard.type("As a customer I want to pay online so that I can order at night");
await feature.getByRole("button", { name: "Add check" }).click();
await page.keyboard.type("Card payments succeed");
await page.keyboard.press("Enter");
await page.keyboard.type("Receipts are emailed");
await feature.getByLabel("Add table").selectOption(orders.id);
const pages = (await call("GET", `/boards/${board.id}/model`)).pages;
await feature.getByLabel("Add page").selectOption(pages[0].children[0].id);
await settle();

const saved = await call("GET", `/items/${featureId}`);
const c = saved.content;
check("status, summary and stories save", c.status === "planned" && c.summary.includes("**checkout**") && c.stories.length === 1, c.status);
check("Enter adds the next acceptance check", c.acceptance.map((a) => a.text).join("|") === "Card payments succeed|Receipts are emailed");
check("tables and pages are linked by id", c.table_ids[0] === orders.id && c.page_ids[0] === pages[0].children[0].id);
check("the summary renders as markdown", (await feature.locator(".feature-summary strong").textContent()) === "checkout");

await feature.locator(".card-header").click();
await page.screenshot({ path: `${OUT}60-feature.png` });

// A table chip pans to the schema card that shows it.
await feature.locator(".chip", { hasText: "orders" }).click();
const selected = await page.locator(".card.is-selected").getAttribute("data-item");
check("a table chip selects the schema card showing it", selected === schemaCard.id);

// ---------- The model, as Design reads it ----------
const res = await fetch(`${BASE}/api/boards/${board.id}/model`, { headers: { Authorization: `Bearer ${token}` } });
const model = await res.json();
const etag = res.headers.get("etag");
check("the model has the feature", model.features[0]?.title === "Feature" && model.features[0].acceptance.length === 2);
check("…with its table and page", model.features[0].table_ids[0] === orders.id && model.features[0].page_ids.length === 1);
const again = await fetch(`${BASE}/api/boards/${board.id}/model`, {
  headers: { Authorization: `Bearer ${token}`, "If-None-Match": etag },
});
check("a second request with the ETag gets 304", again.status === 304, `${again.status}`);
const schema = await fetch(`${BASE}/api/model.schema.json`);
check("the JSON Schema is served without a login", schema.status === 200 && (await schema.text()).includes("BoardModel"));

// ---------- board pull ----------
const dir = mkdtempSync(join(tmpdir(), "board-pull-"));
const pull = () =>
  execFileSync(BIN, ["pull", board.id, "--into", dir, "--from", BASE, "--token", token], { encoding: "utf8" });
const first = pull();
const file = join(dir, "board-model.json");
const before = readFileSync(file, "utf8");
check("board pull writes board-model.json", first.includes("Wrote") && JSON.parse(before).board.title === "Florist site", first.trim());
check("pulling again leaves it alone", pull().includes("up to date"));

// Add an enum value; the file changes in exactly that column.
await call("PATCH", `/tables/${orders.id}`, {
  version: orders.version,
  definition: { columns: [{ name: "id", type: "bigint", primary: true }, { name: "status", type: "enum", values: ["new", "sent", "refunded"] }] },
});
pull();
const after = readFileSync(file, "utf8");
const changed = after.split("\n").filter((line, k) => line !== before.split("\n")[k]);
const added = after.split("\n").length - before.split("\n").length;
check(
  "after a column change the file diffs cleanly",
  added === 1 && after.includes('"refunded"') && changed.some((l) => l.includes('"hash"')),
  `${added} line added; changed: ${changed.map((l) => l.trim()).slice(0, 4).join(" ")}`,
);

check("no page errors", errors.length === 0, errors.join("; "));
await browser.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
