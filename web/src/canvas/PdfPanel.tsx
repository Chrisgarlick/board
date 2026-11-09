// The planning PDF (spec section 8): choose what goes in, check the pages,
// then download it or copy a link. Typeset renders it in the client's brand.

import { useEffect, useMemo, useState } from "preact/hooks";

import { api, ApiError, downloadPdf, type PdfOptions } from "../api";
import { board, frameIds, itemIds, items, report, status } from "../store";
import type { Item } from "../types";

const BAND = 200;
const KIND: Record<string, string> = {
  note: "Note",
  checklist: "Checklist",
  feature: "Feature",
  schema: "Schema",
  sitemap: "Sitemap",
};

/** Frames and loose cards in reading order, as the server sorts them. */
function units(): Item[] {
  const all = [...frameIds.value, ...itemIds.value]
    .map((id) => items.get(id)?.item.value)
    .filter((i): i is Item => !!i && (i.kind === "frame" || (i.kind === "card" && !i.frame_id)));
  return all.sort(
    (a, b) => Math.floor(a.y / BAND) - Math.floor(b.y / BAND) || a.x - b.x || a.id.localeCompare(b.id),
  );
}

export function PdfPanel({ onClose }: { onClose: () => void }) {
  const b = board.value;
  const list = units();
  const [left, setLeft] = useState<ReadonlySet<string>>(new Set());
  const [title, setTitle] = useState(b?.title ?? "");
  const [subtitle, setSubtitle] = useState(b?.description?.split("\n")[0] ?? "");
  const [recipient, setRecipient] = useState("");
  const [clients, setClients] = useState<{ slug: string; name: string }[] | null>(null);
  const [notSetUp, setNotSetUp] = useState<string | null>(null);
  const [busy, setBusy] = useState<"proof" | "pdf" | "link" | null>(null);
  const [proof, setProof] = useState<{ images: string[]; warnings: string[] } | null>(null);

  useEffect(() => {
    api
      .typesetClients()
      .then(setClients)
      .catch((err) => {
        if (err instanceof ApiError && err.body.error === "typeset_not_configured") setNotSetUp(err.body.message);
        else setClients([]);
      });
  }, []);

  const options = useMemo((): PdfOptions => {
    const o: PdfOptions = {};
    const chosen = list.filter((i) => !left.has(i.id)).map((i) => i.id);
    if (chosen.length !== list.length) o.item_ids = chosen;
    if (title.trim() && title.trim() !== b?.title) o.title = title.trim();
    if (subtitle.trim()) o.subtitle = subtitle.trim();
    if (recipient.trim()) o.recipient = recipient.trim();
    return o;
  }, [list.map((i) => i.id).join(), left, title, subtitle, recipient]);

  if (!b) return null;
  const chosenCount = list.length - list.filter((i) => left.has(i.id)).length;

  async function run(kind: "proof" | "pdf" | "link") {
    const current = board.value;
    if (!current) return;
    setBusy(kind);
    try {
      if (kind === "proof") {
        const p = await api.pdfProof(current.id, options);
        setProof({ images: p.images, warnings: p.warnings });
      } else if (kind === "pdf") {
        await downloadPdf(current.id, options);
      } else {
        const { url } = await api.pdfLink(current.id, options);
        await navigator.clipboard.writeText(url);
        status.value = { kind: "info", text: "PDF link copied. It works for 7 days without logging in, and shows the board as it is when opened." };
      }
    } catch (err) {
      report(err, kind === "proof" ? "preview the PDF" : kind === "pdf" ? "make the PDF" : "make a link");
    } finally {
      setBusy(null);
    }
  }

  async function setClient(slug: string) {
    const current = board.value;
    if (!current) return;
    try {
      board.value = await api.updateBoard(current.id, { client: slug || null });
      setProof(null);
    } catch (err) {
      report(err, "set the client");
    }
  }

  function toggle(id: string) {
    const next = new Set(left);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setLeft(next);
    setProof(null);
  }

  return (
    <aside class="panel panel-wide" aria-label="Planning PDF">
      <header class="panel-header">
        <h2>Planning PDF</h2>
        <button type="button" class="icon-button" aria-label="Close PDF" onClick={onClose}>
          ×
        </button>
      </header>
      {notSetUp ? (
        <p class="notice">{notSetUp}</p>
      ) : (
        <>
          <p class="panel-note">
            Frames (their drawing, then the cards in them) and cards outside frames, in reading order. Loose shapes
            and stickies aren't included: put them in a frame.
          </p>
          <form class="settings" onSubmit={(e) => e.preventDefault()}>
            <label>
              Brand
              <select value={b.client ?? ""} onChange={(e) => void setClient(e.currentTarget.value)}>
                <option value="">Typeset's default look</option>
                {(clients ?? []).map((c) => (
                  <option key={c.slug} value={c.slug}>
                    {c.name}
                  </option>
                ))}
                {b.client && clients && !clients.some((c) => c.slug === b.client) && (
                  <option value={b.client}>{b.client}</option>
                )}
              </select>
            </label>
            <label>
              Title
              <input value={title} onInput={(e) => (setTitle(e.currentTarget.value), setProof(null))} />
            </label>
            <label>
              Subtitle
              <input value={subtitle} onInput={(e) => (setSubtitle(e.currentTarget.value), setProof(null))} />
            </label>
            <label>
              For
              <input
                value={recipient}
                placeholder="e.g. Petal & Stem"
                onInput={(e) => (setRecipient(e.currentTarget.value), setProof(null))}
              />
            </label>
          </form>
          <h3 class="pdf-heading">
            Contents <span class="feature-count">{chosenCount} of {list.length}</span>
          </h3>
          {list.length === 0 ? (
            <p class="panel-note">Nothing to put in yet: add a frame or a card.</p>
          ) : (
            <ul class="pdf-units">
              {list.map((i) => (
                <li key={i.id}>
                  <label>
                    <input type="checkbox" checked={!left.has(i.id)} onChange={() => toggle(i.id)} />
                    <span class="pdf-unit-title">{i.title || "Untitled"}</span>
                    <span class="pdf-unit-kind">{i.kind === "frame" ? "Frame" : KIND[i.card_type ?? ""] ?? "Card"}</span>
                  </label>
                </li>
              ))}
            </ul>
          )}
          <div class="pdf-actions">
            <button type="button" disabled={!!busy || chosenCount === 0} onClick={() => void run("proof")}>
              {busy === "proof" ? "Rendering…" : "Preview pages"}
            </button>
            <button type="button" class="primary" disabled={!!busy || chosenCount === 0} onClick={() => void run("pdf")}>
              {busy === "pdf" ? "Rendering…" : "Download PDF"}
            </button>
            <button type="button" class="text-button" disabled={!!busy || chosenCount === 0} onClick={() => void run("link")}>
              Copy link
            </button>
          </div>
          {proof && (
            <>
              {proof.warnings.length > 0 && (
                <ul class="pdf-warnings" role="status">
                  {proof.warnings.map((w) => (
                    <li key={w}>{w}</li>
                  ))}
                </ul>
              )}
              <div class="pdf-proof" aria-label={`${proof.images.length} pages`}>
                {proof.images.map((src, k) => (
                  <img key={k} src={`data:image/png;base64,${src}`} alt={`Page ${k + 1}`} />
                ))}
              </div>
            </>
          )}
        </>
      )}
    </aside>
  );
}
