// Phase 4 browser test: export a frame as PNG and SVG, and share an image link.
// Run with ./scripts/e2e.sh. Usage: node export.mjs <login-link> <admin-token>
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
const pngSize = (buf) => [buf.readUInt32BE(16), buf.readUInt32BE(20)];

const browser = await chromium.launch(exe ? { executablePath: exe } : {});
const ctx = await browser.newContext({ viewport: { width: 1400, height: 900 }, acceptDownloads: true });
await ctx.grantPermissions(["clipboard-read", "clipboard-write"], { origin: BASE });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
page.on("response", (r) => r.status() >= 400 && errors.push(`${r.status()} ${new URL(r.url()).pathname}`));

await page.goto(link);
await page.waitForURL(BASE + "/");
const board = await call("POST", "/boards", { title: "Export board" });
const flow = await call("POST", `/boards/${board.id}/flows`, {
  mermaid: "flowchart LR\n  a(Enquiry) --> b{Quote ok?}\n  b -->|yes| c[Book a call]\n  b -->|no| d[(CRM)]",
  frame_title: "Sales pipeline",
});
const frameId = flow.frames[0];
const frame = await call("GET", `/items/${frameId}`);
await page.goto(`${BASE}/boards/${board.id}`);
await page.locator(".frame-tab").waitFor();
await page.screenshot({ path: `${OUT}30-canvas.png` });

// Select the frame and export it.
await page.locator(".frame-tab").click();
const exportMenu = page.getByRole("toolbar", { name: "Style" }).getByLabel("Export");

const [png] = await Promise.all([page.waitForEvent("download"), exportMenu.selectOption("png")]);
const pngPath = `${OUT}31-export.png`;
await png.saveAs(pngPath);
const pngBytes = readFileSync(pngPath);
const [w, h] = pngSize(pngBytes);
check("Download PNG saves the frame as a PNG", png.suggestedFilename() === "Sales-pipeline.png", png.suggestedFilename());
check("…at 2x, including the title", w === frame.w * 2 && h === (frame.h + 30) * 2, `${w}×${h}`);

const [svg] = await Promise.all([page.waitForEvent("download"), exportMenu.selectOption("svg")]);
const svgText = readFileSync(await svg.path(), "utf8");
check("Download SVG saves an SVG with text as paths", svgText.startsWith("<svg") && !svgText.includes("<text"), `${svgText.length} bytes`);

await exportMenu.selectOption("link");
await page.getByText("Image link copied").waitFor();
const url = await page.evaluate(() => navigator.clipboard.readText());
check("Copy image link puts a signed link on the clipboard", url.startsWith(`${BASE}/i/${frameId}.png?`) && url.includes("sig="), url);

// The link works for someone who isn't logged in.
const stranger = await browser.newContext();
const res = await stranger.request.get(url);
check("…which works without logging in", res.status() === 200 && res.headers()["content-type"] === "image/png", `${res.status()}`);
await stranger.close();

// A selection of two items.
await page.keyboard.press("Escape");
await page.locator(".shape", { hasText: "Enquiry" }).click();
await page.locator(".shape", { hasText: "Book a call" }).click({ modifiers: ["Shift"] });
const [sel] = await Promise.all([page.waitForEvent("download"), exportMenu.selectOption("png")]);
const [sw, sh] = pngSize(readFileSync(await sel.path()));
check("a multi-selection exports cropped to those items", sw > 0 && sw < w && sh < h, `${sw}×${sh}`);

check("no errors", errors.length === 0, errors.join(" | "));
await browser.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
