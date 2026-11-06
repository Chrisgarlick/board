//! The daily housekeeping task: purge old trash and expired sessions.

use std::sync::Arc;
use std::time::Duration;

use sqlx::SqlitePool;

use crate::clock::Clock;
use crate::error::AppResult;
use crate::service::{connectors, items};
use crate::types::{Actor, Ctx};
use crate::{auth, db};

/// Trashed items are kept this long (spec section 4.1).
pub const TRASH_DAYS: i64 = 30;

/// Run once now, then every 24 hours, until the process exits.
pub fn spawn(pool: SqlitePool, clock: Arc<dyn Clock>) {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
        loop {
            every.tick().await;
            if let Err(err) = run(&pool, clock.as_ref()).await {
                tracing::error!(error = ?err, "housekeeping failed");
            }
        }
    });
}

pub async fn run(pool: &SqlitePool, clock: &dyn Clock) -> AppResult<()> {
    let ctx = Ctx {
        actor: Actor::User,
        now: clock.now(),
    };
    let mut tx = db::write_tx(pool).await?;
    let purged = items::purge_trash(&mut tx, ctx, TRASH_DAYS).await?
        + connectors::purge_trash(&mut tx, ctx, TRASH_DAYS).await?;
    tx.commit().await?;
    auth::purge_expired(pool, ctx.now).await?;
    if purged > 0 {
        tracing::info!(purged, "emptied old items from the trash");
    }
    Ok(())
}
