//! Shared source contract used by every data provider (design J2, S9.1).
#![forbid(unsafe_code)]

pub mod contracts;
pub mod pagination;
pub mod resource;

pub use contracts::*;
pub use pagination::*;
pub use resource::*;
