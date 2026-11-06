// Passkeys for signing in (and logging out). Opened from the board list.

import { useEffect, useState } from "preact/hooks";

import { api } from "../api";
import { addPasskey, deviceName, explain, passkeysSupported } from "../passkeys";
import { report, status } from "../store";
import { ago } from "../time";

export interface Passkey {
  id: string;
  name: string;
  created_at: string;
  last_used_at: string | null;
}

export function usePasskeys(): [Passkey[] | null, () => void] {
  const [list, setList] = useState<Passkey[] | null>(null);
  const refresh = () =>
    void api
      .passkeys()
      .then((r) => setList(r.passkeys))
      .catch((err) => report(err, "load your passkeys"));
  useEffect(refresh, []);
  return [list, refresh];
}

export function AccountPanel({
  list,
  refresh,
  onClose,
}: {
  list: Passkey[] | null;
  refresh: () => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(deviceName());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function add(e: Event) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await addPasskey(name.trim() || deviceName());
      status.value = { kind: "info", text: "Passkey added. You can sign in with it from now on." };
      refresh();
    } catch (err) {
      setError(explain(err));
    } finally {
      setBusy(false);
    }
  }

  async function remove(p: Passkey) {
    if (!confirm(`Remove the passkey "${p.name}"? It won't be able to sign in to Board any more.`)) return;
    try {
      await api.removePasskey(p.id);
      refresh();
    } catch (err) {
      report(err, "remove the passkey");
    }
  }

  async function logout() {
    try {
      await api.logout();
    } finally {
      location.reload();
    }
  }

  return (
    <aside class="panel" aria-label="Passkeys">
      <header class="panel-header">
        <h2>Passkeys</h2>
        <button type="button" class="icon-button" aria-label="Close passkeys" onClick={onClose}>
          ×
        </button>
      </header>
      <p class="panel-note">
        Sign in to Board with Face ID, Touch ID, a fingerprint or your device's PIN. Passkeys saved to iCloud Keychain
        or Google Password Manager work on your other devices too.
      </p>
      {list && list.length > 0 && (
        <ul class="trash-list">
          {list.map((p) => (
            <li key={p.id}>
              <div>
                <p class="trash-title">{p.name}</p>
                <p class="trash-meta">
                  Added {ago(p.created_at)}
                  {p.last_used_at ? `, last used ${ago(p.last_used_at)}` : ", not used yet"}
                </p>
              </div>
              <button type="button" class="text-button" onClick={() => void remove(p)}>
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
      {passkeysSupported() ? (
        <form class="passkey-add" onSubmit={add}>
          <label>
            Name
            <input value={name} maxLength={60} onInput={(e) => setName(e.currentTarget.value)} />
          </label>
          <button type="submit" disabled={busy}>
            {busy ? "Waiting for your device…" : "Add a passkey"}
          </button>
          {error && (
            <p class="sign-in-error" role="alert">
              {error}
            </p>
          )}
          <p class="panel-note">
            To put one on your phone from a computer, choose the option to use a phone or tablet when your browser
            asks, and scan the QR code.
          </p>
        </form>
      ) : (
        <p class="panel-note">This browser doesn't support passkeys.</p>
      )}
      <hr class="panel-rule" />
      <button type="button" class="text-button" onClick={() => void logout()}>
        Log out of this browser
      </button>
    </aside>
  );
}
