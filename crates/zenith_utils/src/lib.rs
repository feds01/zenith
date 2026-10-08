//! Various `zenith` utilities, mostly adapted from `bracketlint`: logging,
//! output streams, terminal highlighting, printing helpers, tree writing,
//! timers, line indexing and path helpers.

pub mod glob;
pub mod highlight;
pub mod logging;
pub mod path;
pub mod printing;
pub mod stream;
pub mod text;
mod timers;
pub mod tree_writing;

pub use timers::{format_duration, timed};
