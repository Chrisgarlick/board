// Pan and zoom state for the canvas, and conversions between screen and
// canvas coordinates. The canvas itself is one CSS-transformed element.

import { signal } from "@preact/signals";

export interface View {
  /** Screen position of canvas point (0, 0), in CSS pixels. */
  x: number;
  y: number;
  zoom: number;
}

export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 4;

export const view = signal<View>({ x: 80, y: 80, zoom: 1 });

export function clampZoom(z: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));
}

/** Screen point (relative to the canvas element) to canvas coordinates. */
export function toCanvas(sx: number, sy: number, v: View = view.value): { x: number; y: number } {
  return { x: (sx - v.x) / v.zoom, y: (sy - v.y) / v.zoom };
}

/** Zoom to `zoom`, keeping the canvas point under (sx, sy) where it is. */
export function zoomAt(sx: number, sy: number, zoom: number): void {
  const v = view.value;
  const z = clampZoom(zoom);
  const p = toCanvas(sx, sy, v);
  view.value = { zoom: z, x: sx - p.x * z, y: sy - p.y * z };
}

export function panBy(dx: number, dy: number): void {
  const v = view.value;
  view.value = { ...v, x: v.x + dx, y: v.y + dy };
}

/** Fit a rectangle of canvas space into a `width` × `height` viewport. */
export function fit(
  rect: { x: number; y: number; w: number; h: number },
  width: number,
  height: number,
  padding = 64,
): void {
  const zoom = clampZoom(
    Math.min((width - padding * 2) / rect.w, (height - padding * 2) / rect.h, 1),
  );
  view.value = {
    zoom,
    x: (width - rect.w * zoom) / 2 - rect.x * zoom,
    y: (height - rect.h * zoom) / 2 - rect.y * zoom,
  };
}
