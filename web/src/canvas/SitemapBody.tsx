// A sitemap card (spec 10.3): the site's pages as a nested list. Pages keep
// their ids through every edit (Design keys its frames by them), so new pages
// get an id here, the moment they're added.
//
// Keys: Enter adds a page below, Tab and Shift+Tab nest and un-nest,
// Alt+Up/Down move a page, Backspace on an empty title removes it. Rows can
// also be dragged by their handle.

import { useLayoutEffect, useRef, useState } from "preact/hooks";

import { pageId } from "../ids";
import { editContent, item, rememberEdit, type ContentPatch } from "../store";
import type { SitemapContent, SitemapNode } from "../types";

/** A node's place in the tree: child indexes from the root. The root is []. */
type Path = number[];

function at(root: SitemapNode, path: Path): SitemapNode | undefined {
  let node: SitemapNode | undefined = root;
  for (const k of path) node = node?.children?.[k];
  return node;
}

/** A copy of the tree with the node at `path` replaced by `f(node)`. */
function update(root: SitemapNode, path: Path, f: (n: SitemapNode) => SitemapNode): SitemapNode {
  if (path.length === 0) return f(root);
  const [k, ...rest] = path as [number, ...number[]];
  const children = [...(root.children ?? [])];
  const child = children[k];
  if (!child) return root;
  children[k] = update(child, rest, f);
  return { ...root, children };
}

function removeAt(root: SitemapNode, path: Path): [SitemapNode, SitemapNode | undefined] {
  const parent = path.slice(0, -1);
  const k = path[path.length - 1]!;
  const node = at(root, path);
  return [update(root, parent, (p) => ({ ...p, children: (p.children ?? []).filter((_, j) => j !== k) })), node];
}

function insertAt(root: SitemapNode, parent: Path, index: number, node: SitemapNode): SitemapNode {
  return update(root, parent, (p) => {
    const children = [...(p.children ?? [])];
    children.splice(index, 0, node);
    return { ...p, children };
  });
}

/** Whether `inner` is `outer` or inside it. */
function within(inner: Path, outer: Path): boolean {
  return outer.length <= inner.length && outer.every((k, j) => inner[j] === k);
}

function slugFor(title: string, parentSlug: string | undefined): string {
  const base = title
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
  const prefix = parentSlug && parentSlug !== "/" ? parentSlug : "";
  return `${prefix}/${base}`;
}

export function SitemapBody({ id, content }: { id: string; content: SitemapContent }) {
  const root = content.root;
  const box = useRef<HTMLDivElement>(null);
  const focusNext = useRef<string | null>(null);
  // The content when the current text edit began, for undo.
  const before = useRef<ContentPatch | null>(null);
  const [dragging, setDragging] = useState<Path | null>(null);
  const [drop, setDrop] = useState<{ path: Path; where: "before" | "after" | "inside" } | null>(null);

  useLayoutEffect(() => {
    const target = focusNext.current;
    if (!target) return;
    focusNext.current = null;
    box.current?.querySelector<HTMLInputElement>(`[data-page="${target}"] .page-title`)?.focus();
  });

  function save(next: SitemapNode, typed = false) {
    const patch = { content: { root: next } };
    if (typed) {
      before.current ??= { content: item(id)?.content };
      editContent(id, patch);
    } else {
      const start = { content: item(id)?.content };
      editContent(id, patch, true);
      rememberEdit(id, start, patch);
    }
  }

  function endTyping() {
    const start = before.current;
    before.current = null;
    if (start) rememberEdit(id, start, { content: item(id)?.content });
  }

  function addAfter(path: Path) {
    const page: SitemapNode = { id: pageId(), title: "" };
    if (path.length === 0) {
      save(insertAt(root, [], 0, page));
    } else {
      save(insertAt(root, path.slice(0, -1), path[path.length - 1]! + 1, page));
    }
    focusNext.current = page.id;
  }

  function addChild(path: Path) {
    const page: SitemapNode = { id: pageId(), title: "" };
    const parent = at(root, path);
    save(insertAt(root, path, parent?.children?.length ?? 0, page));
    focusNext.current = page.id;
  }

  function remove(path: Path) {
    const [next, gone] = removeAt(root, path);
    if (gone?.children?.length && !confirm(`Remove "${gone.title || "this page"}" and the pages under it?`)) return;
    save(next);
    const k = path[path.length - 1]!;
    const parent = path.slice(0, -1);
    const neighbour = k > 0 ? at(next, [...parent, k - 1]) : at(next, parent);
    if (neighbour) focusNext.current = neighbour.id;
  }

  /** Move the node at `from` to be child `index` of `parent` (in the tree without it). */
  function move(from: Path, parent: Path, index: number) {
    const [without, node] = removeAt(root, from);
    if (!node) return;
    save(insertAt(without, parent, index, node));
    focusNext.current = node.id;
  }

  function indent(path: Path) {
    const k = path[path.length - 1]!;
    if (k === 0) return; // nothing above to nest under
    const parent = path.slice(0, -1);
    const sibling = at(root, [...parent, k - 1]);
    move(path, [...parent, k - 1], sibling?.children?.length ?? 0);
  }

  function outdent(path: Path) {
    if (path.length < 2) return; // already a top-level page
    const parent = path.slice(0, -1);
    const grand = parent.slice(0, -1);
    // After removing the node, the parent is still at `parent`.
    move(path, grand, parent[parent.length - 1]! + 1);
  }

  function shift(path: Path, by: -1 | 1) {
    const k = path[path.length - 1]!;
    const parent = path.slice(0, -1);
    const count = at(root, parent)?.children?.length ?? 0;
    const to = k + by;
    if (to < 0 || to >= count) return;
    move(path, parent, to);
  }

  function onDrop(target: Path, where: "before" | "after" | "inside") {
    const from = dragging;
    setDragging(null);
    setDrop(null);
    if (!from || within(target, from)) return;
    // Work out the destination in the tree as it will be without the node.
    const [without, node] = removeAt(root, from);
    if (!node) return;
    const moved = (p: Path): Path => {
      // Removing `from` shifts later siblings (and their subtrees) up one.
      const depth = from.length - 1;
      if (p.length > depth && p.slice(0, depth).every((k, j) => k === from[j]) && p[depth]! > from[depth]!) {
        const q = [...p];
        q[depth] = q[depth]! - 1;
        return q;
      }
      return p;
    };
    const t = moved(target);
    let next: SitemapNode;
    if (where === "inside" || t.length === 0) {
      next = insertAt(without, t, at(without, t)?.children?.length ?? 0, node);
    } else {
      const k = t[t.length - 1]!;
      next = insertAt(without, t.slice(0, -1), where === "before" ? k : k + 1, node);
    }
    save(next);
  }

  // A render function, not a component: a component defined in here would be
  // a new type on every render, remounting the rows and losing focus.
  function row(node: SitemapNode, path: Path, parentSlug: string | undefined) {
    const isRoot = path.length === 0;
    const marker = drop && drop.path.join(".") === path.join(".") ? ` drop-${drop.where}` : "";
    return (
      <li key={node.id} class="page" data-page={node.id}>
        <div
          class={`page-row${marker}${dragging && dragging.join(".") === path.join(".") ? " is-dragging" : ""}`}
          onDragOver={(e) => {
            if (!dragging || within(path, dragging)) return;
            e.preventDefault();
            const r = e.currentTarget.getBoundingClientRect();
            const y = (e.clientY - r.top) / r.height;
            const where = isRoot || (y > 0.3 && y < 0.7) ? "inside" : y <= 0.3 ? "before" : "after";
            setDrop({ path, where });
          }}
          onDragLeave={() => setDrop(null)}
          onDrop={(e) => {
            e.preventDefault();
            if (drop) onDrop(path, drop.where);
          }}
        >
          {!isRoot && (
            <span
              class="page-handle"
              draggable
              title="Drag to move"
              aria-hidden="true"
              onDragStart={(e) => {
                e.dataTransfer?.setData("text/plain", node.id);
                setDragging(path);
              }}
              onDragEnd={() => {
                setDragging(null);
                setDrop(null);
              }}
            >
              ⠿
            </span>
          )}
          <input
            class="page-title"
            value={node.title}
            placeholder="Page title"
            aria-label="Page title"
            onInput={(e) => save(update(root, path, (n) => ({ ...n, title: e.currentTarget.value })), true)}
            onBlur={(e) => {
              // A new page gets a slug from its title, once.
              const title = e.currentTarget.value;
              const latest = (item(id)?.content as SitemapContent | undefined)?.root ?? root;
              const current = at(latest, path);
              if (current && !current.slug && title.trim()) {
                const slug = slugFor(title, parentSlug);
                editContent(id, { content: { root: update(latest, path, (n) => ({ ...n, slug })) } });
              }
              endTyping();
            }}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter") {
                e.preventDefault();
                addAfter(path);
              } else if (e.key === "Tab" && !isRoot) {
                e.preventDefault();
                if (e.shiftKey) outdent(path);
                else indent(path);
              } else if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown") && !isRoot) {
                e.preventDefault();
                shift(path, e.key === "ArrowUp" ? -1 : 1);
              } else if (e.key === "Backspace" && node.title === "" && !isRoot) {
                e.preventDefault();
                remove(path);
              }
            }}
          />
          <input
            class="page-slug"
            value={node.slug ?? ""}
            placeholder="/slug"
            aria-label="Slug"
            spellcheck={false}
            onInput={(e) => {
              const slug = e.currentTarget.value;
              save(
                update(root, path, (n) => {
                  const { slug: _old, ...rest } = n;
                  return slug ? { ...rest, slug } : rest;
                }),
                true,
              );
            }}
            onBlur={endTyping}
            onKeyDown={(e) => e.stopPropagation()}
          />
          <span class="page-actions">
            <button type="button" class="icon-button" aria-label="Add a page under this one" title="Add a page under this one" onClick={() => addChild(path)}>
              +
            </button>
            {!isRoot && (
              <button type="button" class="icon-button" aria-label="Remove page" title="Remove page" onClick={() => remove(path)}>
                ×
              </button>
            )}
          </span>
        </div>
        <input
          class={`page-purpose${node.purpose ? " has-purpose" : ""}`}
          value={node.purpose ?? ""}
          placeholder="What this page is for"
          aria-label={`Purpose of ${node.title || "this page"}`}
          onInput={(e) => {
            const purpose = e.currentTarget.value;
            save(
              update(root, path, (n) => {
                const { purpose: _old, ...rest } = n;
                return purpose ? { ...rest, purpose } : rest;
              }),
              true,
            );
          }}
          onBlur={endTyping}
          onKeyDown={(e) => e.stopPropagation()}
        />
        {node.children && node.children.length > 0 && (
          <ul>
            {node.children.map((child, k) => row(child, [...path, k], node.slug))}
          </ul>
        )}
      </li>
    );
  }

  return (
    <div class="sitemap" ref={box} onPointerDown={(e) => e.stopPropagation()}>
      <ul class="sitemap-tree">
        {row(root, [], undefined)}
      </ul>
      <p class="sitemap-hint">Enter adds a page. Tab nests it, Shift+Tab un-nests. Drag the handle to move.</p>
    </div>
  );
}
