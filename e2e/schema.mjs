// Phase 5 browser test: model a database across two schema cards, rename a
// column without its id changing, export the ER diagram, build a sitemap and
// link a journey step to a page, set the stack.
// Run with ./scripts/e2e.sh. Usage: node schema.mjs <login-link> <admin-token>
import { readFileSync } from "node:fs";
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
const call = async (method, path, body) =>
  (
    await fetch(`${BASE}/api${path}`, {
      method,
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: body ? JSON.stringify(body) : undefined,
    })
  ).json();
const settle = () => new Promise((r) => setTimeout(r, 900));
const tableNamed = async (boardId, name) =>
  (await call("GET", `/boards/${boardId}/schema`)).tables.find((t) => t.name === name);

const browser = await chromium.launch(exe ? { executablePath: exe } : {});
const ctx = await browser.newContext({ viewport: { width: 1500, height: 950 }, acceptDownloads: true });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
page.on("response", (r) => r.status() >= 500 && errors.push(`${r.status()} ${new URL(r.url()).pathname}`));
page.on("dialog", (d) => void d.accept());

await page.goto(link);
await page.waitForURL(BASE + "/");
const board = await call("POST", "/boards", { title: "Bakery rebuild" });
await page.goto(`${BASE}/boards/${board.id}`);
await page.locator(".mat").waitFor();

/** Double-click empty canvas at (x, y) and pick from the add menu. */
async function addFromMenu(x, y, label) {
  await page.mouse.dblclick(x, y);
  await page.getByRole("menuitem", { name: label }).click();
}

// ---------- Schema card one: users ----------
await addFromMenu(150, 140, "Schema (tables)");
const people = page.locator(".card-schema").first();
await people.waitFor();
check("the add menu makes a schema card", (await people.locator(".card-kind").textContent()) === "Schema");
await people.getByRole("button", { name: "Add table" }).click();
const nameInput = people.locator(".schema-table-name");
await nameInput.waitFor();
await nameInput.fill("users");
await nameInput.press("Tab");
await people.getByRole("button", { name: "Add column" }).click();
const emailName = people.locator(".col-name").nth(1);
await emailName.fill("email");
await emailName.press("Tab");
await people.getByLabel("Unique").nth(1).check();
await settle();
let users = await tableNamed(board.id, "users");
check("a table is created from the card and named", !!users, users ? users.id : "missing");
check(
  "columns are saved with ids",
  users?.definition.columns.length === 2 && users.definition.columns.every((c) => c.id?.startsWith("col_")),
  JSON.stringify(users?.definition.columns.map((c) => c.name)),
);
check("…and the unique flag", users?.definition.columns[1].unique === true);

// ---------- Schema card two: orders, referencing users ----------
await addFromMenu(900, 140, "Schema (tables)");
const commerce = page.locator(".card-schema").nth(1);
await commerce.waitFor();
await commerce.getByRole("button", { name: "Add table" }).click();
const ordersName = commerce.locator(".schema-table-name");
await ordersName.fill("orders");
await ordersName.press("Tab");
await commerce.getByRole("button", { name: "Add column" }).click();
const fkName = commerce.locator(".col-name").nth(1);
await fkName.fill("user_id");
await fkName.press("Tab");
await settle();
const userPk = users.definition.columns[0];
await commerce.getByLabel("References").nth(1).selectOption(`${users.id}|${userPk.id}`);
await settle();
let orders = await tableNamed(board.id, "orders");
const fk = orders?.definition.columns[1];
check(
  "a foreign key picked across cards points at users.id",
  fk?.references?.table_id === users.id && fk.references.column_id === userPk.id,
  JSON.stringify(fk?.references),
);
check("…and takes the referenced column's type", fk?.type === "bigint", fk?.type);
check("the commerce card shows users as a stub", (await call("GET", `/items/${await commerce.getAttribute("data-item")}/er`)).stubs[0]?.name === "users");

// The ER preview is the server's diagram.
const er = commerce.locator(".er-preview img");
await er.waitFor();
await page.waitForFunction((el) => el.complete && el.naturalWidth > 0, await er.elementHandle());
check("the card shows the ER diagram", (await er.evaluate((img) => img.naturalWidth)) > 100);
await page.screenshot({ path: `${OUT}50-schema-cards.png` });

// ---------- Rename a column: its id stays ----------
const emailId = users.definition.columns[1].id;
const emailInput = people.locator(".col-name").nth(1);
await emailInput.fill("email_address");
await emailInput.press("Tab");
await settle();
users = await tableNamed(board.id, "users");
check(
  "renaming a column keeps its id",
  users.definition.columns[1].name === "email_address" && users.definition.columns[1].id === emailId,
  `${users.definition.columns[1].name} ${users.definition.columns[1].id === emailId ? "same id" : "new id"}`,
);

// Undo puts the old name back (same id).
await page.locator(".mat").click({ position: { x: 700, y: 800 } });
await page.keyboard.press("Meta+z");
await settle();
users = await tableNamed(board.id, "users");
check(
  "undo reverts the rename",
  users.definition.columns[1].name === "email" && users.definition.columns[1].id === emailId,
  users.definition.columns[1].name,
);

// A bad name is explained in place and not saved.
await emailInput.fill("2fa");
await page.getByRole("alert").filter({ hasText: "valid column name" }).waitFor();
check("an invalid column name shows the server's message", true);
await emailInput.fill("email");
await emailInput.press("Tab");
await settle();
check("…and clears once it's fixed", (await people.locator(".schema-error").count()) === 0);

// ---------- Export the ER diagram ----------
await people.locator(".card-header").click();
const exportMenu = page.getByRole("toolbar", { name: "Style" }).getByLabel("Export");
const [png] = await Promise.all([page.waitForEvent("download"), exportMenu.selectOption("png")]);
const bytes = readFileSync(await png.path());
check("Export → PNG on a schema card downloads its ER diagram", bytes.subarray(1, 4).toString() === "PNG", png.suggestedFilename());
await png.saveAs(`${OUT}51-er-export.png`);

// ---------- Schema panel ----------
await page.getByRole("button", { name: "Schema", exact: true }).click();
const panel = page.getByRole("complementary", { name: "Schema" });
await panel.waitFor();
const listed = await panel.locator(".schema-list .trash-title").allTextContents();
check("the schema panel lists every table", listed.join(",") === "orders,users", listed.join(","));
const chips = await panel.locator(".schema-list li").first().locator(".chip").allTextContents();
check("…with the cards that show each one", chips.length === 1, chips.join(","));

// A table added elsewhere (Claude, a script) appears live.
await call("POST", `/boards/${board.id}/tables`, { name: "payments", definition: { columns: [{ name: "id", type: "uuid", primary: true }] } });
await panel.getByText("payments").waitFor();
check("a table added elsewhere appears without a reload", true);
check("…marked as not on any card", (await panel.locator("li", { hasText: "payments" }).textContent()).includes("not on any card"));
await page.screenshot({ path: `${OUT}52-schema-panel.png` });
await panel.getByRole("button", { name: "Close schema" }).click();

// ---------- Sitemap ----------
await addFromMenu(150, 700, "Sitemap");
const sitemap = page.locator(".card-sitemap");
await sitemap.waitFor();
const home = sitemap.locator(".page-title").first();
await home.click();
await home.press("End");
await home.press("Enter");
const second = sitemap.locator(".page-title").nth(1);
await page.waitForFunction((el) => document.activeElement === el, await second.elementHandle());
await page.keyboard.type("Log in");
await page.keyboard.press("Enter");
await page.keyboard.type("Basket");
await sitemap.locator(".page-title").nth(2).blur();
await settle();
const map = await call("GET", `/items/${await sitemap.getAttribute("data-item")}`);
const kids = map.content.root.children ?? [];
check(
  "Enter adds pages, each with a slug from its title",
  kids.map((k) => k.slug).join(",") === "/log-in,/basket",
  kids.map((k) => `${k.title}:${k.slug}`).join(","),
);
check("pages have stable ids", kids.every((k) => k.id.startsWith("pg_")));

// Tab nests Basket under Log in (contrived, but it checks the gesture).
await sitemap.locator(".page-title").nth(2).focus();
await page.keyboard.press("Tab");
await settle();
const nested = (await call("GET", `/items/${map.id}`)).content.root.children;
check("Tab nests a page under the one above", nested.length === 1 && nested[0].children?.[0]?.title === "Basket", JSON.stringify(nested.map((n) => n.title)));
check("…keeping its id", nested[0].children?.[0]?.id === kids[1].id);
await page.keyboard.press("Shift+Tab");
await settle();

// ---------- A journey step linked to a page ----------
const step = await call("POST", `/boards/${board.id}/items`, {
  kind: "shape",
  content: { shape: "rect", text: "Sign in" },
  x: 700,
  y: 760,
});
const shape = page.locator(`[data-item="${step.id}"]`);
await shape.waitFor();
await shape.click();
await page.getByRole("toolbar", { name: "Style" }).getByLabel("Page").selectOption(kids[0].id);
await shape.locator(".shape-page").waitFor();
check("a linked step shows its page's slug", (await shape.locator(".shape-page").textContent()) === "/log-in");
await settle();
check("…and stores page_id", (await call("GET", `/items/${step.id}`)).content.page_id === kids[0].id);
await page.screenshot({ path: `${OUT}53-sitemap.png` });

// ---------- Board settings ----------
await page.getByRole("button", { name: "Settings", exact: true }).click();
const settings = page.getByRole("complementary", { name: "Board settings" });
await settings.getByLabel("Stack").fill("Laravel + MySQL");
await settings.getByLabel("Description").click();
await settle();
check("the stack is saved", (await call("GET", `/boards/${board.id}`)).board.stack === "Laravel + MySQL");

check("no page errors", errors.length === 0, errors.join("; "));
await browser.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
