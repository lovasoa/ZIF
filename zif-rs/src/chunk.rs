use alloc::vec::Vec;
use core::ops::Range;

use crate::tiff::checked_len;
use crate::{Error, Result};

/// A byte-range request made by the parser or by a tile.
///
/// ```
/// let range = zif_tiff::ByteRange::new(10..20)?;
/// assert_eq!(range.range(), 10..20);
/// assert_eq!(range.start(), 10);
/// assert_eq!(range.end(), 20);
/// assert_eq!(range.len(), 10);
/// assert!(!range.is_empty());
/// assert!(zif_tiff::ByteRange::new(4..4)?.is_empty());
/// # Ok::<(), zif_tiff::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteRange {
    range: Range<u64>,
}

impl ByteRange {
    pub fn new(range: Range<u64>) -> Result<Self> {
        if range.start > range.end {
            return Err(Error::InvalidInput("range start is after end"));
        }
        Ok(Self { range })
    }

    pub fn range(&self) -> Range<u64> {
        self.range.clone()
    }

    pub fn start(&self) -> u64 {
        self.range.start
    }

    pub fn end(&self) -> u64 {
        self.range.end
    }

    pub fn len(&self) -> u64 {
        self.range.end - self.range.start
    }

    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }
}

impl From<ByteRange> for Range<u64> {
    fn from(value: ByteRange) -> Self {
        value.range
    }
}

/// A coherent byte payload returned by an IO layer.
///
/// ```
/// let chunk = zif_tiff::DataChunk::new(10..13, vec![1, 2, 3])?;
/// assert_eq!(chunk.range(), 10..13);
/// assert_eq!(chunk.start(), 10);
/// assert_eq!(chunk.end(), 13);
/// assert_eq!(chunk.bytes(), &[1, 2, 3]);
///
/// let chunk = zif_tiff::DataChunk::from_start(20, vec![4, 5])?;
/// assert_eq!(chunk.range(), 20..22);
/// # Ok::<(), zif_tiff::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataChunk<B = Vec<u8>> {
    range: Range<u64>,
    bytes: B,
}

impl Default for DataChunk<Vec<u8>> {
    fn default() -> Self {
        Self {
            range: 0..0,
            bytes: Vec::new(),
        }
    }
}

impl<B: AsRef<[u8]>> DataChunk<B> {
    pub fn new(range: Range<u64>, bytes: B) -> Result<Self> {
        if range.start > range.end {
            return Err(Error::InvalidInput("chunk range start is after end"));
        }
        let expected = range.end - range.start;
        let actual = u64::try_from(bytes.as_ref().len())
            .map_err(|_| Error::InvalidInput("chunk length does not fit u64"))?;
        if expected != actual {
            return Err(Error::InvalidInput(
                "chunk range length differs from byte length",
            ));
        }
        Ok(Self { range, bytes })
    }

    pub fn from_start(start: u64, bytes: B) -> Result<Self> {
        let range = checked_len(start, bytes.as_ref().len())?;
        Self::new(range, bytes)
    }

    pub fn range(&self) -> Range<u64> {
        self.range.clone()
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    pub fn start(&self) -> u64 {
        self.range.start
    }

    pub fn end(&self) -> u64 {
        self.range.end
    }

    pub fn into_parts(self) -> (Range<u64>, B) {
        (self.range, self.bytes)
    }
}
