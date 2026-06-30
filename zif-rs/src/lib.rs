#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(not(test), forbid(unsafe_code))]

extern crate alloc;

#[doc(hidden)]
pub mod doctest;
pub mod error;
mod format;
mod model;
mod reader;
mod writer;

#[cfg(feature = "std")]
pub mod std;

#[cfg(feature = "tokio")]
pub mod tokio;

#[cfg(feature = "reqwest")]
pub mod reqwest;

pub use error::{Error, Result};
pub use model::{ChainKind, Chunk, Codec, ColorModel, Level, Region, Request, Tile, Zif, ZifView};
pub use reader::{ReadStatus, Reader};
pub use writer::{LevelSpec, WriteBatch, WriteOp, Writer, WriterBuilder};
