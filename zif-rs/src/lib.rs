#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(not(test), forbid(unsafe_code))]

extern crate alloc;

mod tiff;

pub mod codec;
pub mod chunk;
pub mod error;
pub mod metadata;
pub mod parser;
mod writer;

#[cfg(feature = "std")]
pub mod std;

#[cfg(feature = "tokio")]
pub mod tokio;

#[cfg(feature = "http")]
pub mod http;

pub mod sample;

pub use chunk::{ByteRange, DataChunk};
pub use codec::{Codec, ColorModel};
pub use error::{Error, Result};
pub use metadata::{Image, ImageKind, Level, Tile, TileIter, View};
pub use parser::{ParseState, Parser};
pub use sample::file as sample_file;
pub use writer::{LevelConfig, WriteAction, WriteBatch, Writer, WriterBuilder};
