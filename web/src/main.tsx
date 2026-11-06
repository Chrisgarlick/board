// The app shell: check the login, then show the board list or a board.

import { render } from "preact";
import { signal } from "@preact/signals";
import { useEffect } from "preact/hooks";

import { api, ApiError } from "./api";
import { BoardList } from "./pages/BoardList";
import { BoardView } from "./pages/BoardView";
import { navigate, path, route } from "./router";
import { status } from "./store";

type Auth = "checking" | "in" | "out" | "offline";
const auth = signal<Auth>("checking");

async function checkAuth(): Promise<void> {
  try {
    await api.me();
    auth.value = "in";
  } catch (err) {
    auth.value = err instanceof ApiError && err.status === 401 ? "out" : "offline";
  }
}

function StatusLine() {
  const s = status.value;
  useEffect(() => {
    if (!s) return;
    const t = setTimeout(() => (status.value = null), s.kind === "error" ? 8000 : 4000);
    return () => clearTimeout(t);
  }, [s]);
  if (!s) return null;
  return (
    <div class={`status status-${s.kind}`} role={s.kind === "error" ? "alert" : "status"}>
      {s.text}
      <button type="button" class="icon-button" aria-label="Dismiss" onClick={() => (status.value = null)}>
        ×
      </button>
    </div>
  );
}

function App() {
  if (auth.value === "checking") return null;
  if (auth.value === "out")
    return (
      <main class="page-message">
        <h1>Log in to Board</h1>
        <p>
          On the server, run <code>board login</code> and open the link it prints. The link works once, within 10
          minutes.
        </p>
      </main>
    );
  if (auth.value === "offline")
    return (
      <main class="page-message">
        <p>Can't reach Board. Check the server is running, then reload.</p>
      </main>
    );

  const r = route(path.value);
  return (
    <>
      {r.page === "boards" && <BoardList />}
      {r.page === "board" && <BoardView id={r.id} />}
      {r.page === "missing" && (
        <main class="page-message">
          <p>There's no page here.</p>
          <button type="button" onClick={() => navigate("/")}>
            Back to boards
          </button>
        </main>
      )}
      <StatusLine />
    </>
  );
}

const root = document.getElementById("app");
if (root) {
  render(<App />, root);
  void checkAuth();
}
