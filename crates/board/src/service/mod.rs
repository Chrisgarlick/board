//! The service layer: every read and write the REST API (and later MCP) makes.

pub mod batch;
pub mod boards;
pub mod connectors;
pub mod frames;
pub mod housekeeping;
pub mod items;
pub mod moves;
pub mod placement;
pub mod search;
