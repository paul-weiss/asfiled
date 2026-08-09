//! Normalization: from raw XBRL observations to a queryable panel.
//!
//! First stage: fiscal calendar alignment (`fiscal`) — derived period
//! identity replaces the misleading `fy`/`fp` tags at ingest time.

pub mod concept_map;
pub mod fiscal;
