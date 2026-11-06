//! Board: a planning canvas for client website projects.
//!
//! See `planning-board-tech-spec-rust.md` for the design and `plan.md` for
//! the build order. The binary (`main.rs`) is a thin wrapper around [`cli`].

pub mod app;
pub mod assets;
pub mod auth;
pub mod cli;
pub mod clock;
pub mod config;
pub mod content;
pub mod db;
pub mod error;
pub mod events;
// The Typeset mapper is wired in during phase 7; until then only its tests use it.
#[allow(dead_code)]
mod export;
pub mod flow;
pub mod mcp;
pub mod render;
pub mod routes;
pub mod schema;
pub mod service;
pub mod signing;
pub mod types;
