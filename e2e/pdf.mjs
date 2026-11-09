// Phase 7 browser test: the planning PDF panel, against a fake Typeset that
// records what Board sends. (The real Typeset's output is checked with its
// own renderer; this checks Board's side end to end.)
// Run with ./scripts/e2e.sh. Usage: node pdf.mjs <login-link> <admin-token>
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
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
  (await fetch(`${BASE}/api${path}`, {
    method,
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  })).json();

// ---------- A fake Typeset ----------
// A 1×1 PNG for proof pages.
const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
const seen = [];
const typeset = createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    seen.push({ path: req.url, auth: req.headers.authorization, body: body ? JSON.parse(body) : null });
    if (req.headers.authorization !== "Bearer ck_e2e") {
      res.writeHead(401, { "Content-Type": "application/json" }).end(JSON.stringify({ error: "bad key", status: 401 }));
    } else if (req.url === "/api/clients") {
      res.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify([{ slug: "petal", name: "Petal & Stem", brand_version: 1 }]));
    } else if (req.url === "/api/proof") {
      const doc = JSON.parse(JSON.parse(body).content);
      const n = doc.pages.length + 1; // a cover, then a page per section (roughly)
      res.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify({ pages: n, ppi: 40, images: Array(n).fill(PNG) }));
    } else {
      res.writeHead(200, { "Content-Type": "application/pdf" }).end("%PDF-1.7 fake");
    }
  });
});
await new Promise((r) => typeset.listen(7171, "127.0.0.1", r));
const last = (path) => [...seen].reverse().find((s) => s.path === path);
const docOf = (path) => JSON.parse(last(path).body.content);

// ---------- A board ----------
const board = await call("POST", "/boards", { title: "Florist site", description: "Same-day delivery" });
await call("POST", `/boards/${board.id}/tables`, { name: "orders", definition: { columns: [{ name: "id", type: "bigint", primary: true }] } });
await call("POST", `/boards/${board.id}/flows`, { mermaid: "flowchart TD\n a[Basket] --> b{Paid?}", frame_title: "Checkout flow" });
await call("POST", `/boards/${board.id}/items`, { kind: "card", card_type: "feature", title: "Online payments", x: 0, y: 1200, content: { status: "planned", acceptance: [{ text: "Cards work" }] } });
await call("POST", `/boards/${board.id}/items`, { kind: "card", card_type: "schema", title: "Data model", x: 600, y: 1200, content: { tables: ["orders"] } });

const browser = await chromium.launch(exe ? { executablePath: exe } : {});
const ctx = await browser.newContext({ viewport: { width: 1500, height: 950 }, acceptDownloads: true });
await ctx.grantPermissions(["clipboard-read", "clipboard-write"], { origin: BASE });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
await page.goto(link);
await page.waitForURL(BASE + "/");
await page.goto(`${BASE}/boards/${board.id}`);
await page.locator(".frame-tab").waitFor();

await page.getByRole("button", { name: "PDF", exact: true }).click();
const panel = page.getByRole("complementary", { name: "Planning PDF" });
await panel.getByRole("option", { name: "Petal & Stem" }).waitFor({ state: "attached" });
const units = await panel.locator(".pdf-unit-title").allTextContents();
check("the panel lists frames and loose cards in reading order", units.join(" | ") === "Checkout flow | Online payments | Data model", units.join(" | "));

// Brand: saved on the board, used for the PDF.
await panel.getByLabel("Brand").selectOption("petal");
await page.waitForTimeout(300);
check("choosing a brand saves the board's client", (await call("GET", `/boards/${board.id}`)).board.client === "petal");

// Leave one out, set the cover, preview.
await panel.getByLabel("For").fill("Petal & Stem");
await panel.locator("li", { hasText: "Online payments" }).getByRole("checkbox").uncheck();
await panel.getByRole("button", { name: "Preview pages" }).click();
await panel.locator(".pdf-proof img").first().waitFor();
const thumbs = await panel.locator(".pdf-proof img").count();
const proofReq = last("/api/proof");
const proofDoc = docOf("/api/proof");
check("Preview shows Typeset's pages", thumbs === proofDoc.pages.length + 1, `${thumbs} pages`);
check("…rendered with Board's key, in the board's brand", proofReq.auth === "Bearer ck_e2e" && proofReq.body.client === "petal");
const openers = proofDoc.pages.flatMap((p) => p.blocks).filter((b) => b.type === "opener").map((b) => b.title);
check("…with only the chosen sections, in order", openers.join(" | ") === "Checkout flow | Data model", openers.join(" | "));
check("…and the cover fields", proofDoc.frontmatter.recipient === "Petal & Stem" && proofDoc.frontmatter.subtitle === "Same-day delivery");
const images = proofDoc.pages.flatMap((p) => p.blocks).filter((b) => b.type === "image");
check("drawings travel inside as vector images", images.length === 2 && images.every((i) => i.url.startsWith("data:image/svg+xml;base64,")), `${images.length}`);
await page.screenshot({ path: `${OUT}80-pdf-panel.png` });

// Download.
const [download] = await Promise.all([page.waitForEvent("download"), panel.getByRole("button", { name: "Download PDF" }).click()]);
const bytes = readFileSync(await download.path(), "utf8");
check("Download PDF saves Typeset's file", bytes.startsWith("%PDF") && download.suggestedFilename() === "Florist-site.pdf", download.suggestedFilename());

// A link anyone can open.
await panel.getByRole("button", { name: "Copy link" }).click();
await page.getByText("PDF link copied").waitFor();
const url = await page.evaluate(() => navigator.clipboard.readText());
const stranger = await browser.newContext();
const res = await stranger.request.get(url);
check("Copy link gives a link that works without logging in", res.status() === 200 && (await res.text()).startsWith("%PDF"), url.slice(0, 60));
const linkDoc = docOf("/api/render");
check("…keeping the same choices", linkDoc.pages.flatMap((p) => p.blocks).filter((b) => b.type === "opener").length === 2);
await stranger.close();

check("no page errors", errors.length === 0, errors.join("; "));
await browser.close();
typeset.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
