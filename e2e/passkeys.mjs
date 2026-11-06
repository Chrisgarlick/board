// Passkey sign-in in a real browser, with Chromium's virtual authenticator
// standing in for Face ID / a fingerprint. Phone-sized, since that's the point.
// Run with ./scripts/e2e.sh. Usage: node passkeys.mjs <login-link> <admin-token>
import { chromium, devices } from "playwright";

const [link] = process.argv.slice(2);
const BASE = "http://localhost:7070";
const OUT = new URL("./screenshots/", import.meta.url).pathname;
const exe = process.env.CHROMIUM_PATH || undefined;

const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  (${detail})` : ""}`);
};

const browser = await chromium.launch(exe ? { executablePath: exe } : {});
// A phone: Pixel 7 size and touch (Chromium can't pretend to be Safari).
const ctx = await browser.newContext({ ...devices["Pixel 7"] });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
page.on("dialog", (d) => void d.accept());

const cdp = await ctx.newCDPSession(page);
await cdp.send("WebAuthn.enable");
const { authenticatorId } = await cdp.send("WebAuthn.addVirtualAuthenticator", {
  options: {
    protocol: "ctap2",
    transport: "internal",
    hasResidentKey: true,
    hasUserVerification: true,
    isUserVerified: true,
    automaticPresenceSimulation: true,
  },
});

// Signed out, the sign-in page offers a passkey.
await page.goto(BASE);
await page.getByRole("button", { name: "Sign in with a passkey" }).waitFor();
await page.screenshot({ path: `${OUT}70-sign-in-phone.png` });
check("signed out, the page offers passkey sign-in", true);

// Without a passkey yet, signing in explains itself.
await page.getByRole("button", { name: "Sign in with a passkey" }).click();
await page.getByRole("alert").waitFor();
check("with no passkey, it says so", (await page.getByRole("alert").textContent()).length > 0, await page.getByRole("alert").textContent());

// First time in: the one-time link, then add a passkey.
await page.goto(link);
await page.waitForURL(BASE + "/");
await page.getByText("Add a passkey so you can sign in").waitFor();
check("the board list nudges you to add a passkey", true);
await page.locator(".notice").getByRole("button", { name: "Add a passkey" }).click();
const panel = page.getByRole("complementary", { name: "Passkeys" });
await panel.getByLabel("Name").fill("Test phone");
await panel.getByRole("button", { name: "Add a passkey" }).click();
await panel.locator(".trash-title", { hasText: "Test phone" }).waitFor();
check("a passkey is added", true);
const { credentials } = await cdp.send("WebAuthn.getCredentials", { authenticatorId });
check("…and lives on the device as a discoverable credential", credentials.length === 1 && credentials[0].isResidentCredential);
await page.screenshot({ path: `${OUT}71-passkeys-panel-phone.png` });

// Log out, then back in with the passkey alone.
await panel.getByRole("button", { name: "Log out of this browser" }).click();
await page.getByRole("button", { name: "Sign in with a passkey" }).waitFor();
check("logging out returns to sign-in", true);
await page.getByRole("button", { name: "Sign in with a passkey" }).click();
await page.getByRole("heading", { name: "Boards" }).waitFor();
check("the passkey signs you back in", true);
const me = await page.evaluate(async () => (await fetch("/api/me")).json());
check("…as you", me.actor === "user", JSON.stringify(me));

// Removing it means it can't sign in any more.
await page.getByRole("button", { name: "Passkeys" }).click();
await page.getByRole("button", { name: "Remove" }).click();
await page.getByText("Add a passkey so you can sign in").waitFor({ state: "hidden" }).catch(() => {});
await page.getByRole("complementary", { name: "Passkeys" }).getByRole("button", { name: "Log out of this browser" }).click();
await page.getByRole("button", { name: "Sign in with a passkey" }).click();
await page.getByRole("alert").waitFor();
check("a removed passkey can't sign in", (await page.getByRole("alert").textContent()).includes("isn't registered"), await page.getByRole("alert").textContent());

check("no page errors", errors.length === 0, errors.join("; "));
await browser.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
