// The REST API client. Every request carries a per-tab client id, so that
// from phase 3 the server can tag live updates with who caused them.

import type { Board, BoardSummary, FullBoard, Item, Revision } from "./types";

const CLIENT_ID = crypto.randomUUID();

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly body: ApiErrorBody,
  ) {
    super(body.message);
  }
}

export interface ApiErrorBody {
  error: string;
  message: string;
  path?: string;
  current?: unknown;
  op_index?: number;
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = { "X-Client-Id": CLIENT_ID };
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const init: RequestInit = { method, headers, credentials: "same-origin" };
  if (body !== undefined) init.body = JSON.stringify(body);

  let res: Response;
  try {
    res = await fetch(`/api${path}`, init);
  } catch {
    throw new ApiError(0, { error: "offline", message: "Can't reach Board. Check your connection." });
  }
  if (res.status === 204) return undefined as T;
  const data: unknown = await res.json().catch(() => null);
  if (!res.ok) {
    const err =
      data && typeof data === "object" && "error" in data
        ? (data as ApiErrorBody)
        : { error: "http", message: `Request failed (${res.status})` };
    throw new ApiError(res.status, err);
  }
  return data as T;
}

export interface ItemPatch {
  version: number;
  title?: string;
  content?: unknown;
  style?: unknown;
}

export interface Geometry {
  x?: number;
  y?: number;
  w?: number;
  h?: number;
  z?: number;
  locked?: boolean;
}

export const api = {
  me: () => request<{ actor: string; scope: string }>("GET", "/me"),
  logout: () => request<void>("POST", "/logout"),

  listBoards: (q: string, archived: boolean) =>
    request<{ boards: BoardSummary[] }>(
      "GET",
      `/boards?archived=${archived}${q ? `&q=${encodeURIComponent(q)}` : ""}`,
    ),
  createBoard: (title: string) => request<Board>("POST", "/boards", { title }),
  updateBoard: (id: string, patch: Partial<Pick<Board, "title" | "archived">>) =>
    request<Board>("PATCH", `/boards/${id}`, patch),
  deleteBoard: (id: string) => request<void>("DELETE", `/boards/${id}`),
  getBoard: (id: string) => request<FullBoard>("GET", `/boards/${id}`),
  trash: (id: string) => request<{ items: Item[] }>("GET", `/boards/${id}/trash`),

  createItem: (boardId: string, item: Record<string, unknown>) =>
    request<Item>("POST", `/boards/${boardId}/items`, item),
  updateItem: (id: string, patch: ItemPatch) => request<Item>("PATCH", `/items/${id}`, patch),
  setGeometry: (id: string, g: Geometry) => request<Item>("PATCH", `/items/${id}/geometry`, g),
  deleteItem: (id: string) => request<void>("DELETE", `/items/${id}`),
  undeleteItem: (id: string) => request<Item>("POST", `/items/${id}/undelete`),
  revisions: (id: string) => request<{ revisions: Revision[] }>("GET", `/items/${id}/revisions`),
};
