//! Moving items to another board (spec section 6, "Moving items between boards").
//!
//! - Shapes, stickies, text, frames (with their members) and cards that don't
//!   refer to their board's tables or pages can move.
//! - Connectors between two moved items move with them; connectors from a
//!   moved item to one that stays behind are deleted.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

use crate::content::CardType;
use crate::error::{AppError, AppResult};
use crate::service::items::{self, Item};
use crate::service::{boards, frames};
use crate::types::Ctx;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveRequest {
    pub item_ids: Vec<String>,
    pub board_id: String,
    #[serde(default)]
    pub dx: f64,
    #[serde(default)]
    pub dy: f64,
}

#[derive(Debug, Serialize)]
pub struct Moved {
    pub from_board: String,
    pub to_board: String,
    /// The moved items, as they are on the new board.
    pub items: Vec<Item>,
    /// Connectors that moved with them.
    pub connectors: Vec<String>,
    /// Connectors deleted because only one end moved.
    pub removed_connectors: Vec<String>,
    /// Items left on the old board whose frame membership changed.
    pub affected: Vec<String>,
}

pub async fn move_items(
    conn: &mut SqliteConnection,
    ctx: Ctx,
    req: MoveRequest,
) -> AppResult<Moved> {
    if req.item_ids.is_empty() {
        return Err(AppError::invalid("item_ids", "nothing to move"));
    }
    if !req.dx.is_finite() || !req.dy.is_finite() {
        return Err(AppError::invalid("dx", "offset must be a number"));
    }
    boards::require_exists(conn, &req.board_id).await?;

    // Expand frames to include their members; every item must be live and on
    // one source board.
    let mut ids: BTreeSet<String> = BTreeSet::new();
    let mut from_board: Option<String> = None;
    for id in &req.item_ids {
        let item = items::get(conn, id).await?;
        match &from_board {
            None => from_board = Some(item.board_id.clone()),
            Some(b) if *b != item.board_id => {
                return Err(AppError::invalid(
                    "item_ids",
                    "items must all come from the same board",
                ));
            }
            _ => {}
        }
        if item.kind == "frame" {
            ids.extend(frames::members(conn, id).await?);
        }
        ids.insert(item.id);
    }
    let from_board = from_board.expect("at least one item");
    if from_board == req.board_id {
        return Err(AppError::invalid(
            "board_id",
            "the items are already on that board",
        ));
    }

    for id in &ids {
        let item = items::get(conn, id).await?;
        if let Some(t) = item.card_type.as_deref()
            && !CardType::parse(t)?.can_move_between_boards()
        {
            return Err(AppError::State {
                code: "cannot_move",
                message: format!(
                    "{t} cards refer to their board's tables or pages and can't move; copy the content instead"
                ),
            });
        }
    }
    items::require_room(conn, &req.board_id, ids.len() as i64).await?;

    // Connectors touching the moved items: keep those with both ends moving.
    let mut moving_connectors = Vec::new();
    let mut removed_connectors = Vec::new();
    let touching: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, from_item, to_item FROM connectors WHERE board_id = ? AND deleted_at IS NULL",
    )
    .bind(&from_board)
    .fetch_all(&mut *conn)
    .await?;
    for (cid, from, to) in touching {
        match (ids.contains(&from), ids.contains(&to)) {
            (true, true) => moving_connectors.push(cid),
            (true, false) | (false, true) => removed_connectors.push(cid),
            (false, false) => {}
        }
    }

    let now = ctx.now.to_db();
    for id in &ids {
        sqlx::query(
            "UPDATE items SET board_id = ?, x = x + ?, y = y + ?, frame_id = NULL, updated_at = ? WHERE id = ?",
        )
        .bind(&req.board_id)
        .bind(req.dx)
        .bind(req.dy)
        .bind(&now)
        .bind(id)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "UPDATE search_fts SET board_id = ? WHERE entity_kind = 'item' AND entity_id = ?",
        )
        .bind(&req.board_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    }
    for cid in &moving_connectors {
        sqlx::query("UPDATE connectors SET board_id = ?, updated_at = ? WHERE id = ?")
            .bind(&req.board_id)
            .bind(&now)
            .bind(cid)
            .execute(&mut *conn)
            .await?;
    }
    for cid in &removed_connectors {
        sqlx::query(
            "UPDATE connectors SET deleted_at = ?, deleted_by = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(ctx.actor.as_str())
        .bind(&now)
        .bind(cid)
        .execute(&mut *conn)
        .await?;
    }

    // Membership on both boards: moved frames take their members along, and
    // items left behind may have lost (or gained) a frame.
    frames::recompute_board(conn, &req.board_id).await?;
    let affected = frames::recompute_board(conn, &from_board).await?;
    boards::touch(conn, &from_board, ctx).await?;
    boards::touch(conn, &req.board_id, ctx).await?;

    let mut moved = Vec::with_capacity(ids.len());
    for id in &ids {
        moved.push(items::get(conn, id).await?);
    }
    Ok(Moved {
        from_board,
        to_board: req.board_id,
        items: moved,
        connectors: moving_connectors,
        removed_connectors,
        affected,
    })
}
