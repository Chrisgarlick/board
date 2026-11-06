// A tiny router: two pages, one signal.

import { signal } from "@preact/signals";

export const path = signal(location.pathname);

window.addEventListener("popstate", () => {
  path.value = location.pathname;
});

export function navigate(to: string): void {
  if (to === location.pathname) return;
  history.pushState(null, "", to);
  path.value = to;
}

export type Route = { page: "boards" } | { page: "board"; id: string } | { page: "missing" };

export function route(p: string): Route {
  if (p === "/" || p === "") return { page: "boards" };
  const m = /^\/boards\/([0-9A-Za-z]+)\/?$/.exec(p);
  if (m?.[1]) return { page: "board", id: m[1] };
  return { page: "missing" };
}
