//! Small types shared across the app: who is calling, and with what rights.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;

/// Who made a change. Comes from the session or token, never the request body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    User,
    Claude,
    Api,
}

impl Actor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Claude => "claude",
            Self::Api => "api",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "claude" => Some(Self::Claude),
            "api" => Some(Self::Api),
            _ => None,
        }
    }
}

impl fmt::Display for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Everything.
    Admin,
    /// GET requests only.
    Read,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Read => "read",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "admin" => Some(Self::Admin),
            "read" => Some(Self::Read),
            _ => None,
        }
    }
}

/// An authenticated caller, attached to each API request by the auth middleware.
#[derive(Debug, Clone, Copy)]
pub struct Caller {
    pub actor: Actor,
    pub scope: Scope,
}

/// What a service call needs to know about the change it's making.
#[derive(Debug, Clone, Copy)]
pub struct Ctx {
    pub actor: Actor,
    pub now: Timestamp,
}

/// A new id: a ULID, sortable by creation time.
pub fn new_id() -> String {
    ulid::Ulid::generate().to_string()
}
