//! Auto-placement for items created without a position (spec section 7).
//!
//! Scan a 4-column grid of 360 × 280 cells, row by row, starting at the
//! top-left of the board's bounding box (or 0,0 on an empty board), and use the
//! first cell where the new item plus a 40-unit margin overlaps nothing.

pub const CELL_W: f64 = 360.0;
pub const CELL_H: f64 = 280.0;
pub const COLUMNS: usize = 4;
pub const MARGIN: f64 = 40.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }

    fn grown(&self, by: f64) -> Rect {
        Rect {
            x: self.x - by,
            y: self.y - by,
            w: self.w + 2.0 * by,
            h: self.h + 2.0 * by,
        }
    }
}

/// Where to put a `w` × `h` item among `existing` ones.
pub fn place(existing: &[Rect], w: f64, h: f64) -> (f64, f64) {
    if existing.is_empty() {
        return (0.0, 0.0);
    }
    let min_x = existing.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
    let min_y = existing.iter().map(|r| r.y).fold(f64::INFINITY, f64::min);
    let max_y = existing
        .iter()
        .map(|r| r.y + r.h)
        .fold(f64::NEG_INFINITY, f64::max);

    for row in 0.. {
        let y = min_y + row as f64 * CELL_H;
        for col in 0..COLUMNS {
            let x = min_x + col as f64 * CELL_W;
            let candidate = Rect { x, y, w, h }.grown(MARGIN);
            if !existing.iter().any(|r| r.overlaps(&candidate)) {
                return (x, y);
            }
        }
        // Below every existing item the first cell is always free; this is
        // just a guard against floating-point surprises.
        if y > max_y + CELL_H {
            return (min_x, y);
        }
    }
    unreachable!("the row loop always returns")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(x: f64, y: f64) -> Rect {
        Rect {
            x,
            y,
            w: 320.0,
            h: 240.0,
        }
    }

    #[test]
    fn empty_board_starts_at_origin() {
        assert_eq!(place(&[], 320.0, 240.0), (0.0, 0.0));
    }

    #[test]
    fn fills_the_first_row_then_wraps() {
        let mut existing = vec![card(0.0, 0.0)];
        assert_eq!(place(&existing, 320.0, 240.0), (360.0, 0.0));
        existing.extend([card(360.0, 0.0), card(720.0, 0.0), card(1080.0, 0.0)]);
        assert_eq!(place(&existing, 320.0, 240.0), (0.0, 280.0));
    }

    #[test]
    fn grid_starts_at_the_bounding_box() {
        let existing = vec![card(1000.0, 500.0)];
        assert_eq!(place(&existing, 320.0, 240.0), (1360.0, 500.0));
    }

    #[test]
    fn skips_cells_blocked_by_big_items() {
        // One wide item covering the first three cells of row 0.
        let existing = vec![Rect {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 200.0,
        }];
        assert_eq!(place(&existing, 320.0, 240.0), (1080.0, 0.0));
    }

    #[test]
    fn never_overlaps() {
        let mut existing = Vec::new();
        for _ in 0..30 {
            let (x, y) = place(&existing, 320.0, 240.0);
            let new = Rect {
                x,
                y,
                w: 320.0,
                h: 240.0,
            };
            assert!(existing.iter().all(|r: &Rect| !r.overlaps(&new)));
            existing.push(new);
        }
    }
}
