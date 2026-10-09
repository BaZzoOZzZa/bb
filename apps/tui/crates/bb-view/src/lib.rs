pub mod activity_intents;
pub mod domain;
pub mod file_change;
pub mod format;
pub mod glyph;
pub mod markdown_split;
pub mod patch;
pub mod path_display;
pub mod shell;
pub mod styled;
pub mod text;
pub mod title;
pub mod view;

pub use text::{TimelineTextOptions, format_thread_timeline_text, format_view_rows_text};
pub use title::{TimelineTitle, TitleOptions, build_timeline_row_title};
pub use view::{ViewRow, build_timeline_view_rows, build_timeline_view_rows_with};
