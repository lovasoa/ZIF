use alloc::vec::Vec;
use core::ops::Range;

use crate::chunk::{ByteRange, DataChunk};
use crate::codec::{Codec, ColorModel};
use crate::metadata::{Image, Level};
use crate::tiff::{
    read_u16, read_u32, read_u64, tile_count, ENTRY_LEN, TAG_BITS, TAG_CHANNELS, TAG_CODEC,
    TAG_COLOR, TAG_HEIGHT, TAG_INTERLEAVE, TAG_TILE_COUNTS, TAG_TILE_HEIGHT, TAG_TILE_OFFSETS,
    TAG_TILE_WIDTH, TAG_WIDTH, TAG_YCBCR_SUBSAMPLING, TYPE_U16, TYPE_U32, TYPE_U64,
};
use crate::{Error, Result};

/// Incremental parsing state returned by [`Parser::feed`].
///
/// ```
/// let mut parser = zif_tiff::Parser::new();
/// let state = parser.feed(zif_tiff::DataChunk::default())?;
/// assert!(matches!(state, zif_tiff::ParseState::Need { .. }));
/// # Ok::<(), zif_tiff::Error>(())
/// ```
#[derive(Debug, Clone)]
pub enum ParseState<'a> {
    Need {
        range: ByteRange,
        partial: Option<&'a Image>,
    },
    Done {
        image: &'a Image,
    },
}

/// Sans-IO ZIF metadata parser.
///
/// Feed byte chunks with [`feed`](Self::feed) and advance the parser
/// incrementally. When the parser needs more data it returns
/// [`ParseState::Need`] with a [`ByteRange`] to fetch. Once the full
/// metadata has been parsed the parser returns [`ParseState::Done`].
///
/// ```
/// let file = zif_tiff::sample::file();
/// let mut parser = zif_tiff::Parser::new();
/// let state = parser.feed(zif_tiff::DataChunk::from_start(0, file)?)?;
/// assert!(matches!(state, zif_tiff::ParseState::Done { .. }));
/// # Ok::<(), zif_tiff::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct Parser {
    cache: Vec<Cached>,
    image: Option<Image>,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    /// Creates a new Sans-IO parser.
    pub fn new() -> Self {
        Self {
            cache: Vec::new(),
            image: None,
        }
    }

    /// Feeds a coherent chunk of bytes to the parser.
    ///
    /// The chunk may be empty, exactly the requested range, a superset of
    /// the requested range, or the whole file.
    pub fn feed<B>(&mut self, chunk: DataChunk<B>) -> Result<ParseState<'_>>
    where
        B: AsRef<[u8]> + Into<Vec<u8>>,
    {
        let (range, bytes) = chunk.into_parts();
        if !bytes.as_ref().is_empty() {
            self.insert_chunk_vec(range, bytes.into())?;
        }
        self.advance()
    }

    fn advance(&mut self) -> Result<ParseState<'_>> {
        match self.try_parse()? {
            Parse::Done(image) => {
                self.image = Some(image);
                Ok(ParseState::Done {
                    image: self.image.as_ref().expect("done parser produced image"),
                })
            }
            Parse::Need { range, partial } => {
                if let Some(image) = partial {
                    self.image = Some(image);
                }
                Ok(ParseState::Need {
                    range: ByteRange::new(range)?,
                    partial: self.image.as_ref(),
                })
            }
        }
    }

    /// Returns the latest parsed image metadata.
    ///
    /// If the previous `feed` returned `Need`, this may be a partial prefix
    /// of the directory chain. After `feed` returns `Done`, this is the
    /// complete file metadata.
    pub fn image(&self) -> Result<&Image> {
        self.image.as_ref().ok_or(Error::Incomplete)
    }

    /// Returns the parsed image metadata, consuming the parser.
    pub fn finish(self) -> Result<Image> {
        self.image.ok_or(Error::Incomplete)
    }

    fn insert_chunk_vec(&mut self, range: Range<u64>, bytes: Vec<u8>) -> Result<()> {
        self.validate_overlap(range.clone(), &bytes)?;
        self.cache.push(Cached {
            start: range.start,
            bytes,
        });
        self.cache.sort_by_key(|c| c.start);
        self.merge_cache();
        Ok(())
    }

    fn validate_overlap(&self, range: Range<u64>, bytes: &[u8]) -> Result<()> {
        for cached in &self.cache {
            let start = range.start.max(cached.start);
            let end = range.end.min(cached.end());
            if start < end {
                let new_start = usize::try_from(start - range.start)
                    .map_err(|_| Error::InvalidInput("chunk overlap too large"))?;
                let old_start = usize::try_from(start - cached.start)
                    .map_err(|_| Error::InvalidInput("cached overlap too large"))?;
                let len = usize::try_from(end - start)
                    .map_err(|_| Error::InvalidInput("overlap too large"))?;
                if bytes[new_start..new_start + len] != cached.bytes[old_start..old_start + len] {
                    return Err(Error::InvalidInput("overlapping chunk bytes differ"));
                }
            }
        }
        Ok(())
    }

    fn merge_cache(&mut self) {
        let mut merged: Vec<Cached> = Vec::new();
        for cached in self.cache.drain(..) {
            if let Some(last) = merged.last_mut() {
                if cached.start <= last.end() {
                    let overlap = usize::try_from(last.end() - cached.start).unwrap_or(usize::MAX);
                    if overlap < cached.bytes.len() {
                        last.bytes.extend_from_slice(&cached.bytes[overlap..]);
                    }
                    continue;
                }
            }
            merged.push(cached);
        }
        self.cache = merged;
    }

    fn bytes(&self, range: Range<u64>) -> Option<&[u8]> {
        self.cache.iter().find_map(|c| {
            if range.start >= c.start && range.end <= c.end() {
                let start = usize::try_from(range.start - c.start).ok()?;
                let end = usize::try_from(range.end - c.start).ok()?;
                Some(&c.bytes[start..end])
            } else {
                None
            }
        })
    }

    fn require(&self, range: Range<u64>) -> core::result::Result<&[u8], Range<u64>> {
        self.bytes(range.clone()).ok_or(range)
    }

    fn try_parse(&self) -> Result<Parse> {
        let header = match self.require(0..16) {
            Ok(h) => h,
            Err(r) => return Ok(need(r, Vec::new())),
        };
        let mut first8 = [0u8; 8];
        first8.copy_from_slice(&header[..8]);
        if first8 != [0x49, 0x49, 0x2b, 0x00, 0x08, 0x00, 0x00, 0x00] {
            return Err(Error::MalformedFile("invalid header"));
        }
        let mut dir = read_u64(header, 8)?;
        if dir == 0 {
            return Err(Error::MalformedFile("missing first directory"));
        }

        let mut levels = Vec::new();
        let mut visited = Vec::new();
        while dir != 0 {
            if visited.contains(&dir) {
                return Err(Error::MalformedFile("directory cycle"));
            }
            visited.push(dir);
            let count_range = dir..dir
                .checked_add(8)
                .ok_or(Error::MalformedFile("directory range overflow"))?;
            let count_bytes = match self.require(count_range) {
                Ok(b) => b,
                Err(r) => return Ok(need(r, levels)),
            };
            let entry_count = read_u64(count_bytes, 0)?;
            if entry_count > 4096 {
                return Err(Error::MalformedFile("too many directory entries"));
            }
            let body_len = 8u64
                .checked_add(
                    entry_count
                        .checked_mul(ENTRY_LEN as u64)
                        .ok_or(Error::MalformedFile("directory size overflow"))?,
                )
                .and_then(|v| v.checked_add(8))
                .ok_or(Error::MalformedFile("directory size overflow"))?;
            let body_range = dir..dir
                .checked_add(body_len)
                .ok_or(Error::MalformedFile("directory range overflow"))?;
            let body = match self.require(body_range) {
                Ok(b) => b,
                Err(r) => return Ok(need(r, levels)),
            };
            let (entries, next) = parse_entries(body, entry_count)?;
            let level = match self.parse_level(levels.len(), &entries)? {
                LevelParse::Done(level) => level,
                LevelParse::Need(range) => return Ok(need(range, levels)),
            };
            levels.push(level);
            dir = next;
        }
        if levels.is_empty() {
            return Err(Error::MalformedFile("no levels"));
        }
        Ok(Parse::Done(Image::new(levels)))
    }

    fn parse_level(&self, index: usize, entries: &[Entry]) -> Result<LevelParse> {
        let width = scalar_u32_or_u16(entries, TAG_WIDTH)?;
        let height = scalar_u32_or_u16(entries, TAG_HEIGHT)?;
        let codec = Codec::from_code(scalar_u16(entries, TAG_CODEC)?)?;
        let color_model = ColorModel::from_code(scalar_u16(entries, TAG_COLOR)?)?;
        let channels = scalar_u16(entries, TAG_CHANNELS)?;
        if channels != 1 && channels != 3 {
            return Err(Error::MalformedFile("invalid channels"));
        }
        self.validate_bits(entries, channels)?;
        match (channels, color_model) {
            (1, ColorModel::WhiteIsZero | ColorModel::BlackIsZero)
            | (3, ColorModel::Rgb | ColorModel::YCbCr) => {}
            _ => {
                return Err(Error::MalformedFile(
                    "color model does not match channel count",
                ))
            }
        }
        if scalar_u16(entries, TAG_INTERLEAVE)? != 1 {
            return Err(Error::MalformedFile("invalid interleave"));
        }
        let tile_width = scalar_u32_or_u16(entries, TAG_TILE_WIDTH)?;
        let tile_height = scalar_u32_or_u16(entries, TAG_TILE_HEIGHT)?;
        if width == 0 || height == 0 || tile_width == 0 || tile_height == 0 {
            return Err(Error::MalformedFile("zero dimension"));
        }
        if tile_width % 16 != 0 || tile_height % 16 != 0 {
            return Err(Error::MalformedFile("tile size is not a multiple of 16"));
        }
        let (_, _, tile_count) = tile_count(width, height, tile_width, tile_height)?;
        let offsets_entry = find(entries, TAG_TILE_OFFSETS)?;
        let counts_entry = find(entries, TAG_TILE_COUNTS)?;
        if offsets_entry.count != tile_count {
            return Err(Error::MalformedFile("invalid tile offsets entry"));
        }
        if counts_entry.count != tile_count {
            return Err(Error::MalformedFile("invalid tile byte counts entry"));
        }
        let offsets = match self.read_offset_array(offsets_entry)? {
            ArrayParse::Done(v) => v,
            ArrayParse::Need(r) => return Ok(LevelParse::Need(r)),
        };
        let counts = match self.read_count_array(counts_entry)? {
            ArrayParse::Done(v) => v,
            ArrayParse::Need(r) => return Ok(LevelParse::Need(r)),
        };
        let ycbcr_subsampling = if codec == Codec::Jpeg && color_model == ColorModel::YCbCr {
            match find_optional(entries, TAG_YCBCR_SUBSAMPLING) {
                Some(entry) => {
                    if entry.ty != TYPE_U16 || entry.count != 2 {
                        return Err(Error::MalformedFile("invalid YCbCr subsampling entry"));
                    }
                    let values = match self.read_u16_array(entry)? {
                        ArrayParse::Done(v) => v,
                        ArrayParse::Need(r) => return Ok(LevelParse::Need(r)),
                    };
                    Some((values[0], values[1]))
                }
                None => None,
            }
        } else {
            None
        };
        for (&offset, &count) in offsets.iter().zip(&counts) {
            offset
                .checked_add(u64::from(count))
                .ok_or(Error::MalformedFile("tile byte range overflows"))?;
        }
        if self.is_known_complete_file(&offsets, &counts) {
            let file_len = self.cache.iter().find(|c| c.start == 0).unwrap().end();
            for (&offset, &count) in offsets.iter().zip(&counts) {
                if offset + u64::from(count) > file_len {
                    return Err(Error::MalformedFile("tile byte range exceeds file length"));
                }
            }
        }
        Ok(LevelParse::Done(Level::new(
            index,
            width,
            height,
            tile_width,
            tile_height,
            codec,
            color_model,
            channels,
            ycbcr_subsampling,
            offsets,
            counts,
        )?))
    }

    fn validate_bits(&self, entries: &[Entry], channels: u16) -> Result<()> {
        let entry = find(entries, TAG_BITS)?;
        if entry.ty != TYPE_U16 || (entry.count != 1 && entry.count != u64::from(channels)) {
            return Err(Error::MalformedFile("invalid bit depth entry"));
        }
        let bits = match self.read_u16_array(entry)? {
            ArrayParse::Done(v) => v,
            ArrayParse::Need(_) => return Err(Error::MalformedFile("invalid bit depth entry")),
        };
        if bits.iter().all(|&bits| bits == 8) {
            Ok(())
        } else {
            Err(Error::MalformedFile("invalid bit depth"))
        }
    }

    fn is_known_complete_file(&self, offsets: &[u64], counts: &[u32]) -> bool {
        let Some(prefix) = self.cache.iter().find(|c| c.start == 0) else {
            return false;
        };
        let prefix_len = prefix.end();
        let max_tile_end = offsets
            .iter()
            .zip(counts)
            .map(|(&offset, &count)| offset.saturating_add(u64::from(count)))
            .max()
            .unwrap_or(0);
        max_tile_end <= prefix_len
    }

    fn read_u64_array(&self, entry: &Entry) -> Result<ArrayParse<u64>> {
        if entry.count == 1 {
            return Ok(ArrayParse::Done(alloc::vec![read_u64(&entry.slot, 0)?]));
        }
        let len = entry
            .count
            .checked_mul(8)
            .ok_or(Error::MalformedFile("array length overflow"))?;
        let offset = read_u64(&entry.slot, 0)?;
        let range = offset
            ..offset
                .checked_add(len)
                .ok_or(Error::MalformedFile("array range overflow"))?;
        let bytes = match self.require(range) {
            Ok(b) => b,
            Err(r) => return Ok(ArrayParse::Need(r)),
        };
        let mut out = Vec::new();
        for i in 0..entry.count {
            out.push(read_u64(
                bytes,
                usize::try_from(i * 8).map_err(|_| Error::MalformedFile("array index overflow"))?,
            )?);
        }
        Ok(ArrayParse::Done(out))
    }

    fn read_offset_array(&self, entry: &Entry) -> Result<ArrayParse<u64>> {
        match entry.ty {
            TYPE_U16 => self.read_u16_as_u64_array(entry),
            TYPE_U32 => self.read_u32_as_u64_array(entry),
            TYPE_U64 => self.read_u64_array(entry),
            _ => Err(Error::MalformedFile("invalid tile offsets entry")),
        }
    }

    fn read_count_array(&self, entry: &Entry) -> Result<ArrayParse<u32>> {
        match entry.ty {
            TYPE_U16 => self.read_u16_as_u32_array(entry),
            TYPE_U32 => self.read_u32_array(entry),
            TYPE_U64 => self.read_u64_as_u32_array(entry),
            _ => Err(Error::MalformedFile("invalid tile byte counts entry")),
        }
    }

    fn read_u16_array(&self, entry: &Entry) -> Result<ArrayParse<u16>> {
        if entry.count <= 4 {
            let mut out = Vec::new();
            for i in 0..entry.count {
                out.push(read_u16(
                    &entry.slot,
                    usize::try_from(i * 2)
                        .map_err(|_| Error::MalformedFile("array index overflow"))?,
                )?);
            }
            return Ok(ArrayParse::Done(out));
        }
        let len = entry
            .count
            .checked_mul(2)
            .ok_or(Error::MalformedFile("array length overflow"))?;
        let offset = read_u64(&entry.slot, 0)?;
        let range = offset
            ..offset
                .checked_add(len)
                .ok_or(Error::MalformedFile("array range overflow"))?;
        let bytes = match self.require(range) {
            Ok(b) => b,
            Err(r) => return Ok(ArrayParse::Need(r)),
        };
        let mut out = Vec::new();
        for i in 0..entry.count {
            out.push(read_u16(
                bytes,
                usize::try_from(i * 2).map_err(|_| Error::MalformedFile("array index overflow"))?,
            )?);
        }
        Ok(ArrayParse::Done(out))
    }

    fn read_u16_as_u32_array(&self, entry: &Entry) -> Result<ArrayParse<u32>> {
        match self.read_u16_array(entry)? {
            ArrayParse::Done(values) => Ok(ArrayParse::Done(
                values.into_iter().map(u32::from).collect(),
            )),
            ArrayParse::Need(range) => Ok(ArrayParse::Need(range)),
        }
    }

    fn read_u16_as_u64_array(&self, entry: &Entry) -> Result<ArrayParse<u64>> {
        match self.read_u16_array(entry)? {
            ArrayParse::Done(values) => Ok(ArrayParse::Done(
                values.into_iter().map(u64::from).collect(),
            )),
            ArrayParse::Need(range) => Ok(ArrayParse::Need(range)),
        }
    }

    fn read_u32_as_u64_array(&self, entry: &Entry) -> Result<ArrayParse<u64>> {
        match self.read_u32_array(entry)? {
            ArrayParse::Done(values) => Ok(ArrayParse::Done(
                values.into_iter().map(u64::from).collect(),
            )),
            ArrayParse::Need(range) => Ok(ArrayParse::Need(range)),
        }
    }

    fn read_u64_as_u32_array(&self, entry: &Entry) -> Result<ArrayParse<u32>> {
        match self.read_u64_array(entry)? {
            ArrayParse::Done(values) => values
                .into_iter()
                .map(|value| {
                    u32::try_from(value)
                        .map_err(|_| Error::MalformedFile("tile byte count exceeds u32"))
                })
                .collect::<Result<Vec<_>>>()
                .map(ArrayParse::Done),
            ArrayParse::Need(range) => Ok(ArrayParse::Need(range)),
        }
    }

    fn read_u32_array(&self, entry: &Entry) -> Result<ArrayParse<u32>> {
        if entry.count <= 2 {
            let mut out = Vec::new();
            for i in 0..entry.count {
                out.push(read_u32(
                    &entry.slot,
                    usize::try_from(i * 4)
                        .map_err(|_| Error::MalformedFile("array index overflow"))?,
                )?);
            }
            return Ok(ArrayParse::Done(out));
        }
        let len = entry
            .count
            .checked_mul(4)
            .ok_or(Error::MalformedFile("array length overflow"))?;
        let offset = read_u64(&entry.slot, 0)?;
        let range = offset
            ..offset
                .checked_add(len)
                .ok_or(Error::MalformedFile("array range overflow"))?;
        let bytes = match self.require(range) {
            Ok(b) => b,
            Err(r) => return Ok(ArrayParse::Need(r)),
        };
        let mut out = Vec::new();
        for i in 0..entry.count {
            out.push(read_u32(
                bytes,
                usize::try_from(i * 4).map_err(|_| Error::MalformedFile("array index overflow"))?,
            )?);
        }
        Ok(ArrayParse::Done(out))
    }
}

#[derive(Debug, Clone)]
struct Cached {
    start: u64,
    bytes: Vec<u8>,
}

impl Cached {
    fn end(&self) -> u64 {
        self.start + u64::try_from(self.bytes.len()).unwrap_or(u64::MAX)
    }
}

enum Parse {
    Need {
        range: Range<u64>,
        partial: Option<Image>,
    },
    Done(Image),
}

fn need(range: Range<u64>, levels: Vec<Level>) -> Parse {
    Parse::Need {
        range,
        partial: (!levels.is_empty()).then(|| Image::new(levels)),
    }
}

enum LevelParse {
    Need(Range<u64>),
    Done(Level),
}

enum ArrayParse<T> {
    Need(Range<u64>),
    Done(Vec<T>),
}

#[derive(Clone)]
struct Entry {
    code: u16,
    ty: u16,
    count: u64,
    slot: [u8; 8],
}

fn parse_entries(bytes: &[u8], count: u64) -> Result<(Vec<Entry>, u64)> {
    let mut out = Vec::new();
    let mut prev = None;
    for i in 0..count {
        let off = 8 + usize::try_from(i)
            .map_err(|_| Error::MalformedFile("entry index overflow"))?
            * ENTRY_LEN;
        let code = read_u16(bytes, off)?;
        if prev.is_some_and(|p| code <= p) {
            return Err(Error::MalformedFile(
                "directory entries are not strictly sorted",
            ));
        }
        prev = Some(code);
        let ty = read_u16(bytes, off + 2)?;
        let count = read_u64(bytes, off + 4)?;
        let mut slot = [0u8; 8];
        slot.copy_from_slice(&bytes[off + 12..off + 20]);
        out.push(Entry {
            code,
            ty,
            count,
            slot,
        });
    }
    let next = read_u64(
        bytes,
        8 + usize::try_from(count).map_err(|_| Error::MalformedFile("entry count overflow"))?
            * ENTRY_LEN,
    )?;
    Ok((out, next))
}

fn find(entries: &[Entry], code: u16) -> Result<&Entry> {
    find_optional(entries, code).ok_or(Error::MalformedFile("missing required entry"))
}

fn find_optional(entries: &[Entry], code: u16) -> Option<&Entry> {
    entries.iter().find(|e| e.code == code)
}

fn scalar_u16(entries: &[Entry], code: u16) -> Result<u16> {
    let e = find(entries, code)?;
    if e.ty != TYPE_U16 || e.count != 1 {
        return Err(Error::MalformedFile("invalid scalar u16 entry"));
    }
    read_u16(&e.slot, 0)
}

fn scalar_u32_or_u16(entries: &[Entry], code: u16) -> Result<u64> {
    let e = find(entries, code)?;
    if e.count != 1 {
        return Err(Error::MalformedFile("invalid scalar entry count"));
    }
    match e.ty {
        TYPE_U16 => Ok(u64::from(read_u16(&e.slot, 0)?)),
        TYPE_U32 => Ok(u64::from(read_u32(&e.slot, 0)?)),
        _ => Err(Error::MalformedFile("invalid scalar entry type")),
    }
}
