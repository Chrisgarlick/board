// Live updates (spec section 6): one EventSource per open board. Changes
// made by this tab come back tagged with its client id and are skipped;
// everything else is applied. After a reconnect, or when the server says we
// fell behind ("resync"), the board is reloaded rather than patched.

import { CLIENT_ID } from "./api";
import { applyRemote, reload, type RemoteChange } from "./store";

const CHANGE_EVENTS = [
  "batch",
  "item.created",
  "item.updated",
  "item.geometry",
  "item.deleted",
  "item.undeleted",
  "item.moved",
  "connector.created",
  "connector.updated",
  "connector.deleted",
  "board.updated",
  "board.deleted",
];

/** Start listening to a board. Returns a function that stops. */
export function listen(boardId: string): () => void {
  const source = new EventSource(`/api/boards/${boardId}/events`);
  let connected = false;

  source.addEventListener("ready", () => {
    // The first connection follows a fresh load; any later one is a reconnect
    // after a gap, so catch up.
    if (connected) void reload();
    connected = true;
  });

  source.addEventListener("resync", () => void reload());

  const onChange = (e: MessageEvent<string>) => {
    try {
      const data = JSON.parse(e.data) as { origin: string | null; changes: RemoteChange[] };
      if (data.origin === CLIENT_ID) return;
      if (data.changes.some((c) => c.type === "resync")) {
        void reload();
        return;
      }
      applyRemote(data.changes);
    } catch {
      void reload();
    }
  };
  for (const name of [...CHANGE_EVENTS, "resync"]) source.addEventListener(name, onChange);

  return () => source.close();
}
