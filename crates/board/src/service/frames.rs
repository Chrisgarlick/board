//! Frame membership (spec section 4.5).
//!
//! The server decides which frame an item belongs to, so every client gets the
//! same answer: an item's `frame_id` is the topmost live frame (highest `z`)
//! whose rectangle contains the item's centre, or none. Frames don't nest.

use sqlx::SqliteConnection;

use crate::error::AppResult;

#[derive(Debug, Clone, PartialEq)]
pub struct FrameRect {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub z: i64,
}

/// The frame containing the point (`cx`, `cy`), if any. Pure, so it's easy to
/// test and to mirror in the UI.
pub fn containing(frames: &[FrameRect], cx: f64, cy: f64) -> Option<&str> {
    frames
        .iter()
        .filter(|f| cx >= f.x && cx <= f.x + f.w && cy >= f.y && cy <= f.y + f.h)
        // Topmost wins; ties go to the newer frame (ids sort by creation).
        .max_by(|a, b| a.z.cmp(&b.z).then_with(|| a.id.cmp(&b.id)))
        .map(|f| f.id.as_str())
}

async fn live_frames(conn: &mut SqliteConnection, board_id: &str) -> AppResult<Vec<FrameRect>> {
    let rows: Vec<(String, f64, f64, f64, f64, i64)> = sqlx::query_as(
        "SELECT id, x, y, w, h, z FROM items
         WHERE board_id = ? AND kind = 'frame' AND deleted_at IS NULL",
    )
    .bind(board_id)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, x, y, w, h, z)| FrameRect { id, x, y, w, h, z })
        .collect())
}

/// Recompute `frame_id` for every live non-frame item on a board. Returns the
/// ids of items whose membership changed.
pub async fn recompute_board(
    conn: &mut SqliteConnection,
    board_id: &str,
) -> AppResult<Vec<String>> {
    let frames = live_frames(conn, board_id).await?;
    let items: Vec<(String, f64, f64, f64, f64, Option<String>)> = sqlx::query_as(
        "SELECT id, x, y, w, h, frame_id FROM items
         WHERE board_id = ? AND kind <> 'frame' AND deleted_at IS NULL",
    )
    .bind(board_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut changed = Vec::new();
    for (id, x, y, w, h, current) in items {
        let target = containing(&frames, x + w / 2.0, y + h / 2.0);
        if target != current.as_deref() {
            set_frame(conn, &id, target).await?;
            changed.push(id);
        }
    }
    Ok(changed)
}

/// (board_id, kind, x, y, w, h, frame_id) for one item.
type Placement = (String, String, f64, f64, f64, f64, Option<String>);

/// Recompute `frame_id` for one item. Returns true if it changed.
pub async fn recompute_item(conn: &mut SqliteConnection, item_id: &str) -> AppResult<bool> {
    let row: Option<Placement> = sqlx::query_as(
        "SELECT board_id, kind, x, y, w, h, frame_id FROM items WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(item_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((board_id, kind, x, y, w, h, current)) = row else {
        return Ok(false);
    };
    if kind == "frame" {
        return Ok(false);
    }
    let frames = live_frames(conn, &board_id).await?;
    let target = containing(&frames, x + w / 2.0, y + h / 2.0);
    if target == current.as_deref() {
        return Ok(false);
    }
    set_frame(conn, item_id, target).await?;
    Ok(true)
}

async fn set_frame(conn: &mut SqliteConnection, id: &str, frame: Option<&str>) -> AppResult<()> {
    sqlx::query("UPDATE items SET frame_id = ? WHERE id = ?")
        .bind(frame)
        .bind(id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Live members of a frame.
pub async fn members(conn: &mut SqliteConnection, frame_id: &str) -> AppResult<Vec<String>> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM items WHERE frame_id = ? AND deleted_at IS NULL")
            .bind(frame_id)
            .fetch_all(conn)
            .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: &str, x: f64, y: f64, w: f64, h: f64, z: i64) -> FrameRect {
        FrameRect {
            id: id.into(),
            x,
            y,
            w,
            h,
            z,
        }
    }

    #[test]
    fn centre_decides_and_topmost_wins() {
        let frames = vec![
            frame("a", 0.0, 0.0, 400.0, 400.0, 1),
            frame("b", 200.0, 200.0, 400.0, 400.0, 5),
        ];
        assert_eq!(containing(&frames, 100.0, 100.0), Some("a"));
        assert_eq!(
            containing(&frames, 300.0, 300.0),
            Some("b"),
            "overlap: higher z"
        );
        assert_eq!(containing(&frames, 700.0, 700.0), None);
        assert_eq!(
            containing(&frames, 400.0, 100.0),
            Some("a"),
            "edges count as inside"
        );
    }
}
