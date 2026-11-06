// The app shell: check the login, then show the board list or a board.

import { render } from "preact";
import { signal } from "@preact/signals";
import { useEffect, useState } from "preact/hooks";

import { api, ApiError } from "./api";
import { explain, passkeysSupported, signInWithPasskey } from "./passkeys";
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

function SignIn() {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function go() {
    setBusy(true);
    setError(null);
    try {
      await signInWithPasskey();
      await checkAuth();
    } catch (err) {
      setError(explain(err));
    } finally {
      setBusy(false);
    }
  }
  return (
    <main class="sign-in">
      <h1>Board</h1>
      {passkeysSupported() ? (
        <button type="button" class="sign-in-button" disabled={busy} onClick={() => void go()}>
          {busy ? "Waiting for your passkey…" : "Sign in with a passkey"}
        </button>
      ) : (
        <p>This browser doesn't support passkeys.</p>
      )}
      {error && (
        <p class="sign-in-error" role="alert">
          {error}
        </p>
      )}
      <p class="sign-in-help">
        No passkey yet? Run <code>board login</code> on the server and open the link it prints (it works once, within
        10 minutes), then add a passkey from the board list.
      </p>
    </main>
  );
}

function App() {
  if (auth.value === "checking") return null;
  if (auth.value === "out") return <SignIn />;
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
