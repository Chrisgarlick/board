//! Live updates (spec section 6, "Live updates (SSE)").
//!
//! Every write publishes the changes it made to the board's broadcast channel;
//! `GET /api/boards/{id}/events` streams them to subscribers as server-sent
//! events. Each event is `{ origin, changes: [...] }`; its SSE name is the
//! change type for a single change, or `batch` for several, so a client can
//! apply a whole batch, frame move or flow in one go.
//!
//! There's no replay. A subscriber that falls behind gets `resync` and should
//! reload the board; clients also reload on every (re)connect.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::service::connectors::Connector;
use crate::service::items::Item;

/// Events buffered per subscriber before it counts as lagging.
const CAPACITY: usize = 512;

/// One change to a board.
#[derive(Debug, Clone, Serialize)]
pub struct Change {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Item>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector: Option<Connector>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board: Option<Value>,
}

impl Change {
    pub fn item(kind: &'static str, item: Item) -> Self {
        Self {
            kind,
            item: Some(item),
            connector: None,
            id: None,
            board: None,
        }
    }

    pub fn connector(kind: &'static str, connector: Connector) -> Self {
        Self {
            kind,
            item: None,
            connector: Some(connector),
            id: None,
            board: None,
        }
    }

    pub fn removed(kind: &'static str, id: String) -> Self {
        Self {
            kind,
            item: None,
            connector: None,
            id: Some(id),
            board: None,
        }
    }

    pub fn board(kind: &'static str, board: Value) -> Self {
        Self {
            kind,
            item: None,
            connector: None,
            id: None,
            board: Some(board),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Published {
    /// The `X-Client-Id` of the request that caused it, if any.
    pub origin: Option<String>,
    pub changes: Vec<Change>,
}

/// Broadcast channels, one per board with subscribers.
#[derive(Clone, Default)]
pub struct Hub {
    boards: Arc<Mutex<HashMap<String, broadcast::Sender<Arc<Published>>>>>,
}

impl Hub {
    /// Publish changes to a board's subscribers. Does nothing if there are none.
    pub fn publish(&self, board_id: &str, origin: Option<String>, changes: Vec<Change>) {
        if changes.is_empty() {
            return;
        }
        let mut boards = self.boards.lock().expect("hub lock");
        if let Some(tx) = boards.get(board_id)
            && tx.send(Arc::new(Published { origin, changes })).is_err()
        {
            // Everyone unsubscribed.
            boards.remove(board_id);
        }
    }

    pub fn subscribe(&self, board_id: &str) -> broadcast::Receiver<Arc<Published>> {
        let mut boards = self.boards.lock().expect("hub lock");
        boards
            .entry(board_id.to_string())
            .or_insert_with(|| broadcast::channel(CAPACITY).0)
            .subscribe()
    }

    /// The SSE response for one subscriber.
    pub fn stream(
        &self,
        board_id: &str,
    ) -> Sse<impl Stream<Item = Result<Event, Infallible>> + use<>> {
        let mut rx = self.subscribe(board_id);
        let stream = async_stream::stream! {
            yield Ok(Event::default().event("ready").data("{}"));
            loop {
                match rx.recv().await {
                    Ok(published) => {
                        let name = match published.changes.as_slice() {
                            [one] => one.kind,
                            _ => "batch",
                        };
                        let data = serde_json::to_string(&*published).unwrap_or_else(|_| "{}".into());
                        yield Ok(Event::default().event(name).data(data));
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        yield Ok(Event::default().event("resync").data(json!({}).to_string()));
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        };
        Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribers_get_published_changes() {
        let hub = Hub::default();
        let mut rx = hub.subscribe("b1");
        hub.publish(
            "b1",
            Some("tab-1".into()),
            vec![Change::removed("item.deleted", "i1".into())],
        );
        hub.publish(
            "other",
            None,
            vec![Change::removed("item.deleted", "x".into())],
        );
        let got = rx.recv().await.unwrap();
        assert_eq!(got.origin.as_deref(), Some("tab-1"));
        assert_eq!(got.changes[0].id.as_deref(), Some("i1"));
        assert!(rx.try_recv().is_err(), "nothing from other boards");
    }

    #[tokio::test]
    async fn a_slow_subscriber_is_told_to_resync() {
        let hub = Hub::default();
        let mut rx = hub.subscribe("b1");
        for n in 0..(CAPACITY + 5) {
            hub.publish(
                "b1",
                None,
                vec![Change::removed("item.deleted", n.to_string())],
            );
        }
        assert!(matches!(
            rx.recv().await,
            Err(broadcast::error::RecvError::Lagged(_))
        ));
    }
}
