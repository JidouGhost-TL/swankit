//! Save media and integrity rules. Byte access and edits use Formatkit.
#![forbid(unsafe_code)]

pub mod checksum;
mod media;
mod owner;

pub use media::{EepromSize, Media, SaveImage};
pub use owner::{Birthday, OwnerProfile};
