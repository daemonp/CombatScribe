//! Data structures for parsed combat log data.
//!
//! Full port of the `logData` global object from `app.js`, plus
//! encounter filtering helpers.

mod event_log;
mod filtering;
mod timeline;
mod timeline_builder;
mod types;

pub use event_log::*;
pub use timeline::*;
pub use types::*;
// filtering.rs and timeline_builder.rs add impl blocks to LogData —
// they compile automatically, no re-export needed.
