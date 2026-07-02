#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(not(test), forbid(unsafe_code))]

extern crate alloc;

pub mod error;
mod format;
mod model;
mod reader;
pub mod sample;
mod writer;

#[cfg(feature = "std")]
pub mod std;

#[cfg(feature = "tokio")]
pub mod tokio;

#[cfg(feature = "http")]
pub mod http;

pub use error::{Error, Result};
pub use model::{ChainKind, Chunk, Codec, ColorModel, Level, Region, Request, Tile, Zif, ZifView};
pub use reader::{ReadStatus, Reader};
pub use sample::zif_bytes as sample_zif_bytes;
pub use writer::{LevelSpec, WriteBatch, WriteOp, Writer, WriterBuilder};
