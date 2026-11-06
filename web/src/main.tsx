// Placeholder app. The board list and canvas arrive in phase 1.
import { render } from "preact";
import { signal } from "@preact/signals";

interface Health {
  status: string;
  version: string;
}

type HealthState =
  | { kind: "loading" }
  | { kind: "ok"; health: Health }
  | { kind: "error"; message: string };

const health = signal<HealthState>({ kind: "loading" });

async function loadHealth(): Promise<void> {
  try {
    const res = await fetch("/healthz");
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    health.value = { kind: "ok", health: (await res.json()) as Health };
  } catch (err) {
    health.value = { kind: "error", message: String(err) };
  }
}

function App() {
  const h = health.value;
  return (
    <main class="app">
      <h1>Board</h1>
      <p class="status">
        {h.kind === "loading" && "Connecting…"}
        {h.kind === "ok" && `Server ${h.health.status}, version ${h.health.version}.`}
        {h.kind === "error" && `Can't reach the server: ${h.message}`}
      </p>
    </main>
  );
}

const root = document.getElementById("app");
if (root) {
  render(<App />, root);
  void loadHealth();
}
