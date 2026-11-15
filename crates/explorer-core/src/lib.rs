//! エクスプローラーのUI非依存コア。GPUIの型はここに持ち込まない。

pub mod entry;
pub mod format;
pub mod history;
pub mod listing;
pub mod location;
pub mod platform;

pub use entry::{Entry, EntryKind, SortKey, SortOrder};
pub use history::History;
pub use listing::{CancelToken, ListingEvent, ListingStats, list_dir};

pub mod date_group;

pub mod wsl;
pub use wsl::{WslDistribution, installed_distributions};
