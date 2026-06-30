use alloc::vec::Vec;
use core::num::NonZeroU64;

use crate::format::{
    push_u16, push_u32, push_u64, tile_count, ENTRY_LEN, TAG_BITS, TAG_CHANNELS, TAG_CODEC,
    TAG_COLOR, TAG_HEIGHT, TAG_INTERLEAVE, TAG_TILE_COUNTS, TAG_TILE_HEIGHT, TAG_TILE_OFFSETS,
    TAG_TILE_WIDTH, TAG_WIDTH, TAG_YCBCR_SUBSAMPLING, TYPE_U16, TYPE_U32, TYPE_U64,
};
use crate::model::{Codec, ColorModel};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelSpec {
    dimensions: (u64, u64),
    tile_size: (u32, u32),
}

impl LevelSpec {
    /// Creates a level specification from image dimensions and tile size.
    ///
    /// ```
    /// let level = zif::LevelSpec::new((1024, 768), (256, 256))?;
    /// let _ = level;
    /// # Ok::<(), zif::Error>(())
    /// ```
    pub fn new(dimensions: (u64, u64), tile_size: (u32, u32)) -> Result<Self> {
        validate_level_spec(dimensions, tile_size)?;
        Ok(Self {
            dimensions,
            tile_size,
        })
    }
}

#[derive(Debug, Clone)]
pub struct WriterBuilder {
    dimensions: Option<(u64, u64)>,
    tile_size: Option<(u32, u32)>,
    levels: Vec<LevelSpec>,
    codec: Option<Codec>,
    color_model: Option<ColorModel>,
    channels: Option<u16>,
    ycbcr_subsampling: Option<(u16, u16)>,
}

impl WriterBuilder {
    /// Sets the dimensions for a single-level writer.
    ///
    /// ```
    /// let builder = zif::Writer::new().dimensions((1024, 768));
    /// let _ = builder;
    /// ```
    pub fn dimensions(mut self, dimensions: (u64, u64)) -> Self {
        self.dimensions = Some(dimensions);
        self
    }
    /// Sets the tile size for a single-level writer.
    ///
    /// ```
    /// let builder = zif::Writer::new()
    ///     .dimensions((1024, 768))
    ///     .tile_size((256, 256))?;
    /// let _ = builder;
    /// # Ok::<(), zif::Error>(())
    /// ```
    pub fn tile_size(mut self, tile_size: (u32, u32)) -> Result<Self> {
        self.tile_size = Some(tile_size);
        Ok(self)
    }
    /// Adds an explicit level to the writer.
    ///
    /// ```
    /// let builder = zif::Writer::new()
    ///     .level(zif::LevelSpec::new((1024, 768), (256, 256))?);
    /// let _ = builder;
    /// # Ok::<(), zif::Error>(())
    /// ```
    pub fn level(mut self, level: LevelSpec) -> Self {
        self.levels.push(level);
        self
    }
    /// Sets the tile codec recorded in generated directories.
    ///
    /// ```
    /// let builder = zif::Writer::new().codec(zif::Codec::Jpeg);
    /// let _ = builder;
    /// ```
    pub fn codec(mut self, codec: Codec) -> Self {
        self.codec = Some(codec);
        self
    }
    /// Sets the color model recorded in generated directories.
    ///
    /// ```
    /// let builder = zif::Writer::new().color_model(zif::ColorModel::YCbCr);
    /// let _ = builder;
    /// ```
    pub fn color_model(mut self, color_model: ColorModel) -> Self {
        self.color_model = Some(color_model);
        self
    }
    /// Sets the channel count.
    ///
    /// ```
    /// let builder = zif::Writer::new().channels(3)?;
    /// let _ = builder;
    /// # Ok::<(), zif::Error>(())
    /// ```
    pub fn channels(mut self, channels: u16) -> Result<Self> {
        validate_channels(channels)?;
        self.channels = Some(channels);
        Ok(self)
    }

    /// Sets a spec-conforming YCbCr subsampling value for JPEG YCbCr tiles.
    ///
    /// ```
    /// let builder = zif::Writer::new().ycbcr_subsampling((2, 2))?;
    /// let _ = builder;
    /// # Ok::<(), zif::Error>(())
    /// ```
    pub fn ycbcr_subsampling(mut self, subsampling: (u16, u16)) -> Result<Self> {
        validate_ycbcr_subsampling(subsampling)?;
        self.ycbcr_subsampling = Some(subsampling);
        Ok(self)
    }

    /// Preserves a nonstandard YCbCr subsampling value while rewriting an existing file.
    ///
    /// This is a compatibility escape hatch for files described in the non-conformance
    /// note in specification section 6.6. New files should use [`Self::ycbcr_subsampling`].
    pub fn preserve_nonstandard_ycbcr_subsampling(
        mut self,
        subsampling: (u16, u16),
    ) -> Result<Self> {
        validate_nonstandard_ycbcr_subsampling(subsampling)?;
        self.ycbcr_subsampling = Some(subsampling);
        Ok(self)
    }

    /// Builds a Sans-IO writer.
    ///
    /// ```
    /// let mut writer = zif::Writer::new()
    ///     .dimensions((16, 16))
    ///     .tile_size((16, 16))?
    ///     .codec(zif::Codec::Jpeg)
    ///     .color_model(zif::ColorModel::YCbCr)
    ///     .channels(3)?
    ///     .build()?;
    /// let _ = writer.put_tile((0, 0), b"jpeg")?;
    /// # Ok::<(), zif::Error>(())
    /// ```
    pub fn build(mut self) -> Result<Writer> {
        if self.levels.is_empty() {
            let dimensions = self
                .dimensions
                .ok_or(Error::InvalidInput("missing dimensions"))?;
            let tile_size = self
                .tile_size
                .ok_or(Error::InvalidInput("missing tile size"))?;
            self.levels.push(LevelSpec::new(dimensions, tile_size)?);
        }
        let codec = self.codec.ok_or(Error::InvalidInput("missing codec"))?;
        let color_model = self
            .color_model
            .ok_or(Error::InvalidInput("missing color model"))?;
        let channels = self
            .channels
            .ok_or(Error::InvalidInput("missing channels"))?;
        validate_color_channels(color_model, channels)?;
        let mut levels = Vec::new();
        for spec in self.levels {
            let (across, down, count) = tile_count(
                spec.dimensions.0,
                spec.dimensions.1,
                u64::from(spec.tile_size.0),
                u64::from(spec.tile_size.1),
            )?;
            let len = usize::try_from(count)
                .map_err(|_| Error::Unsupported("too many tiles for in-memory writer"))?;
            levels.push(LevelState {
                spec,
                tiles_across: across,
                tiles_down: down,
                offsets: alloc::vec![0; len],
                counts: alloc::vec![0; len],
            });
        }
        Ok(Writer {
            levels,
            codec,
            color_model,
            channels,
            ycbcr_subsampling: self.ycbcr_subsampling.unwrap_or((2, 2)),
            file_len: 0,
            initialized: false,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Writer {
    levels: Vec<LevelState>,
    codec: Codec,
    color_model: ColorModel,
    channels: u16,
    ycbcr_subsampling: (u16, u16),
    file_len: u64,
    initialized: bool,
}

impl Writer {
    /// Starts building a writer.
    ///
    /// ```
    /// let builder = zif::Writer::new();
    /// let _ = builder;
    /// ```
    pub fn new() -> WriterBuilder {
        WriterBuilder {
            dimensions: None,
            tile_size: None,
            levels: Vec::new(),
            codec: None,
            color_model: None,
            channels: None,
            ycbcr_subsampling: None,
        }
    }

    /// Adds or replaces a tile at level 0.
    ///
    /// ```
    /// let mut writer = zif::Writer::new()
    ///     .dimensions((16, 16))
    ///     .tile_size((16, 16))?
    ///     .codec(zif::Codec::Jpeg)
    ///     .color_model(zif::ColorModel::YCbCr)
    ///     .channels(3)?
    ///     .build()?;
    /// let batch = writer.put_tile((0, 0), b"jpeg")?;
    /// assert!(!batch.is_empty());
    /// # Ok::<(), zif::Error>(())
    /// ```
    pub fn put_tile(&mut self, coord: (u64, u64), bytes: impl AsRef<[u8]>) -> Result<WriteBatch> {
        self.put_tile_at_level(0, coord, bytes)
    }

    /// Adds or replaces a tile at a specific level.
    ///
    /// ```
    /// let mut writer = zif::Writer::new()
    ///     .level(zif::LevelSpec::new((16, 16), (16, 16))?)
    ///     .codec(zif::Codec::Jpeg)
    ///     .color_model(zif::ColorModel::YCbCr)
    ///     .channels(3)?
    ///     .build()?;
    /// let batch = writer.put_tile_at_level(0, (0, 0), b"jpeg")?;
    /// assert!(!batch.is_empty());
    /// # Ok::<(), zif::Error>(())
    /// ```
    pub fn put_tile_at_level(
        &mut self,
        level: usize,
        coord: (u64, u64),
        bytes: impl AsRef<[u8]>,
    ) -> Result<WriteBatch> {
        let bytes = bytes.as_ref();
        if bytes.len() > u32::MAX as usize {
            return Err(Error::InvalidInput("tile payload exceeds u32::MAX"));
        }
        let mut levels = self.levels.clone();
        let state = levels
            .get_mut(level)
            .ok_or(Error::InvalidInput("level index out of range"))?;
        let (col, row) = coord;
        if col >= state.tiles_across || row >= state.tiles_down {
            return Err(Error::InvalidInput("tile coordinate out of range"));
        }
        let index = usize::try_from(row * state.tiles_across + col)
            .map_err(|_| Error::InvalidInput("tile index too large"))?;

        let mut batch = WriteBatch { ops: Vec::new() };
        if !self.initialized {
            batch.ops.push(WriteOp::InitHeader([
                0x49, 0x49, 0x2b, 0x00, 0x08, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
            ]));
            self.file_len = 16;
            self.initialized = true;
        }
        let tile_offset = self.reserve(bytes.len())?;
        batch.ops.push(WriteOp::Append(bytes.to_vec()));
        state.offsets[index] = tile_offset;
        state.counts[index] = u32::try_from(bytes.len())
            .map_err(|_| Error::InvalidInput("tile payload exceeds u32::MAX"))?;
        let (metadata, first_dir) = self.encode_metadata(&levels)?;
        self.file_len = self
            .file_len
            .checked_add(
                u64::try_from(metadata.len())
                    .map_err(|_| Error::InvalidInput("metadata length too large"))?,
            )
            .ok_or(Error::InvalidInput("file length overflow"))?;
        batch.ops.push(WriteOp::Append(metadata));
        batch.ops.push(WriteOp::PatchU64 {
            offset: NonZeroU64::new(8).expect("8 is non-zero"),
            value: first_dir,
        });
        self.levels = levels;
        Ok(batch)
    }

    /// Changes level-0 dimensions for a single-level writer.
    ///
    /// ```
    /// let mut writer = zif::Writer::new()
    ///     .dimensions((16, 16))
    ///     .tile_size((16, 16))?
    ///     .codec(zif::Codec::Jpeg)
    ///     .color_model(zif::ColorModel::YCbCr)
    ///     .channels(3)?
    ///     .build()?;
    /// let batch = writer.set_dimensions((32, 16))?;
    /// assert!(!batch.is_empty());
    /// # Ok::<(), zif::Error>(())
    /// ```
    pub fn set_dimensions(&mut self, dimensions: (u64, u64)) -> Result<WriteBatch> {
        if self.levels.len() != 1 {
            return Err(Error::Unsupported(
                "set_dimensions currently supports one-level writers",
            ));
        }
        let tile_size = self.levels[0].spec.tile_size;
        validate_level_spec(dimensions, tile_size)?;
        let mut levels = self.levels.clone();
        let old = &self.levels[0];
        let spec = LevelSpec::new(dimensions, tile_size)?;
        let (across, down, count) = tile_count(
            dimensions.0,
            dimensions.1,
            u64::from(tile_size.0),
            u64::from(tile_size.1),
        )?;
        let mut next = LevelState {
            spec,
            tiles_across: across,
            tiles_down: down,
            offsets: alloc::vec![0; usize::try_from(count).map_err(|_| Error::Unsupported("too many tiles"))?],
            counts: alloc::vec![0; usize::try_from(count).map_err(|_| Error::Unsupported("too many tiles"))?],
        };
        let copy_rows = old.tiles_down.min(down);
        let copy_cols = old.tiles_across.min(across);
        for row in 0..copy_rows {
            for col in 0..copy_cols {
                let old_i = usize::try_from(row * old.tiles_across + col)
                    .map_err(|_| Error::InvalidInput("tile index too large"))?;
                let new_i = usize::try_from(row * across + col)
                    .map_err(|_| Error::InvalidInput("tile index too large"))?;
                next.offsets[new_i] = old.offsets[old_i];
                next.counts[new_i] = old.counts[old_i];
            }
        }
        levels[0] = next;
        let mut batch = WriteBatch { ops: Vec::new() };
        if !self.initialized {
            batch.ops.push(WriteOp::InitHeader([
                0x49, 0x49, 0x2b, 0x00, 0x08, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
            ]));
            self.file_len = 16;
            self.initialized = true;
        }
        let (metadata, first_dir) = self.encode_metadata(&levels)?;
        self.file_len = self
            .file_len
            .checked_add(
                u64::try_from(metadata.len())
                    .map_err(|_| Error::InvalidInput("metadata length too large"))?,
            )
            .ok_or(Error::InvalidInput("file length overflow"))?;
        batch.ops.push(WriteOp::Append(metadata));
        batch.ops.push(WriteOp::PatchU64 {
            offset: NonZeroU64::new(8).expect("8 is non-zero"),
            value: first_dir,
        });
        self.levels = levels;
        Ok(batch)
    }

    fn reserve(&mut self, len: usize) -> Result<u64> {
        let offset = self.file_len;
        self.file_len = self
            .file_len
            .checked_add(u64::try_from(len).map_err(|_| Error::InvalidInput("length too large"))?)
            .ok_or(Error::InvalidInput("file length overflow"))?;
        Ok(offset)
    }

    fn encode_metadata(&self, levels: &[LevelState]) -> Result<(Vec<u8>, u64)> {
        let base = self.file_len;
        let mut out = Vec::new();
        let mut dirs = Vec::new();
        for level in levels {
            let offsets_pos = if level.offsets.len() > 1 {
                let pos = base
                    + u64::try_from(out.len())
                        .map_err(|_| Error::InvalidInput("metadata too large"))?;
                for v in &level.offsets {
                    push_u64(&mut out, *v);
                }
                Some(pos)
            } else {
                None
            };
            let counts_pos = if level.counts.len() > 2 {
                let pos = base
                    + u64::try_from(out.len())
                        .map_err(|_| Error::InvalidInput("metadata too large"))?;
                for v in &level.counts {
                    push_u32(&mut out, *v);
                }
                Some(pos)
            } else {
                None
            };
            dirs.push(DirPlan {
                offsets_pos,
                counts_pos,
            });
        }
        let mut dir_offsets = Vec::new();
        for (i, level) in levels.iter().enumerate() {
            let dir_offset = base
                + u64::try_from(out.len())
                    .map_err(|_| Error::InvalidInput("metadata too large"))?;
            dir_offsets.push(dir_offset);
            let next = 0; // temporary
            let dir = encode_directory(
                level,
                dirs[i].offsets_pos,
                dirs[i].counts_pos,
                next,
                self.codec,
                self.color_model,
                self.channels,
                self.ycbcr_subsampling,
            )?;
            out.extend_from_slice(&dir);
        }
        let mut rebuilt = out;
        // Directories are last and fixed-size, so patch their trailing next-dir fields in memory.
        let mut cursor = 0usize;
        for level in levels {
            if level.offsets.len() > 1 {
                cursor += level.offsets.len() * 8;
            }
            if level.counts.len() > 2 {
                cursor += level.counts.len() * 4;
            }
        }
        for i in 0..levels.len() {
            let dir_start = cursor;
            let entry_count = directory_entry_count(self.codec, self.color_model);
            let next_pos = dir_start + 8 + entry_count * ENTRY_LEN;
            let next = if i + 1 < dir_offsets.len() {
                dir_offsets[i + 1]
            } else {
                0
            };
            rebuilt[next_pos..next_pos + 8].copy_from_slice(&next.to_le_bytes());
            cursor = next_pos + 8;
        }
        Ok((rebuilt, dir_offsets[0]))
    }
}

#[derive(Debug, Clone)]
struct LevelState {
    spec: LevelSpec,
    tiles_across: u64,
    tiles_down: u64,
    offsets: Vec<u64>,
    counts: Vec<u32>,
}
struct DirPlan {
    offsets_pos: Option<u64>,
    counts_pos: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteBatch {
    ops: Vec<WriteOp>,
}

impl WriteBatch {
    /// Returns the ordered write operations in this batch.
    ///
    /// ```
    /// let mut writer = zif::Writer::new()
    ///     .dimensions((16, 16))
    ///     .tile_size((16, 16))?
    ///     .codec(zif::Codec::Jpeg)
    ///     .color_model(zif::ColorModel::YCbCr)
    ///     .channels(3)?
    ///     .build()?;
    /// let batch = writer.put_tile((0, 0), b"jpeg")?;
    /// assert!(!batch.ops().is_empty());
    /// # Ok::<(), zif::Error>(())
    /// ```
    pub fn ops(&self) -> &[WriteOp] {
        &self.ops
    }
    /// Consumes the batch and returns its ordered operations.
    ///
    /// ```
    /// let mut writer = zif::Writer::new()
    ///     .dimensions((16, 16))
    ///     .tile_size((16, 16))?
    ///     .codec(zif::Codec::Jpeg)
    ///     .color_model(zif::ColorModel::YCbCr)
    ///     .channels(3)?
    ///     .build()?;
    /// let ops = writer.put_tile((0, 0), b"jpeg")?.into_ops();
    /// assert!(!ops.is_empty());
    /// # Ok::<(), zif::Error>(())
    /// ```
    pub fn into_ops(self) -> Vec<WriteOp> {
        self.ops
    }
    /// Returns true when this batch has no operations.
    ///
    /// ```
    /// let mut writer = zif::Writer::new()
    ///     .dimensions((16, 16))
    ///     .tile_size((16, 16))?
    ///     .codec(zif::Codec::Jpeg)
    ///     .color_model(zif::ColorModel::YCbCr)
    ///     .channels(3)?
    ///     .build()?;
    /// assert!(!writer.put_tile((0, 0), b"jpeg")?.is_empty());
    /// # Ok::<(), zif::Error>(())
    /// ```
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOp {
    InitHeader([u8; 16]),
    Append(Vec<u8>),
    PatchU64 { offset: NonZeroU64, value: u64 },
}

fn validate_level_spec(dimensions: (u64, u64), tile_size: (u32, u32)) -> Result<()> {
    if dimensions.0 == 0 || dimensions.1 == 0 || tile_size.0 == 0 || tile_size.1 == 0 {
        return Err(Error::InvalidInput("zero dimension"));
    }
    if tile_size.0 % 16 != 0 || tile_size.1 % 16 != 0 {
        return Err(Error::InvalidInput("tile size must be a multiple of 16"));
    }
    tile_count(
        dimensions.0,
        dimensions.1,
        u64::from(tile_size.0),
        u64::from(tile_size.1),
    )?;
    Ok(())
}

fn validate_channels(channels: u16) -> Result<()> {
    if channels == 1 || channels == 3 {
        Ok(())
    } else {
        Err(Error::InvalidInput("channels must be 1 or 3"))
    }
}

fn validate_color_channels(color: ColorModel, channels: u16) -> Result<()> {
    match (channels, color) {
        (1, ColorModel::WhiteIsZero | ColorModel::BlackIsZero)
        | (3, ColorModel::Rgb | ColorModel::YCbCr) => Ok(()),
        _ => Err(Error::InvalidInput("color model does not match channels")),
    }
}

fn validate_ycbcr_subsampling(subsampling: (u16, u16)) -> Result<()> {
    if subsampling == (1, 1) || subsampling == (2, 2) {
        return Ok(());
    }
    Err(Error::InvalidInput("unsupported YCbCr subsampling"))
}

fn validate_nonstandard_ycbcr_subsampling(subsampling: (u16, u16)) -> Result<()> {
    if subsampling.0 == 0 || subsampling.1 == 0 {
        return Err(Error::InvalidInput(
            "YCbCr subsampling factors must be non-zero",
        ));
    }
    Ok(())
}

fn encode_directory(
    level: &LevelState,
    offsets_pos: Option<u64>,
    counts_pos: Option<u64>,
    next: u64,
    codec: Codec,
    color: ColorModel,
    channels: u16,
    ycbcr_subsampling: (u16, u16),
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let has_ycbcr_subsampling = codec == Codec::Jpeg && color == ColorModel::YCbCr;
    push_u64(
        &mut out,
        u64::try_from(directory_entry_count(codec, color)).unwrap(),
    );
    entry_u32(
        &mut out,
        TAG_WIDTH,
        u32::try_from(level.spec.dimensions.0)
            .map_err(|_| Error::Unsupported("width exceeds u32"))?,
    );
    entry_u32(
        &mut out,
        TAG_HEIGHT,
        u32::try_from(level.spec.dimensions.1)
            .map_err(|_| Error::Unsupported("height exceeds u32"))?,
    );
    entry_u16(&mut out, TAG_BITS, 8);
    entry_u16(&mut out, TAG_CODEC, codec.code());
    entry_u16(&mut out, TAG_COLOR, color.code());
    entry_u16(&mut out, TAG_CHANNELS, channels);
    entry_u16(&mut out, TAG_INTERLEAVE, 1);
    entry_u32(&mut out, TAG_TILE_WIDTH, level.spec.tile_size.0);
    entry_u32(&mut out, TAG_TILE_HEIGHT, level.spec.tile_size.1);
    entry_u64_array(&mut out, TAG_TILE_OFFSETS, &level.offsets, offsets_pos)?;
    entry_u32_array(&mut out, TAG_TILE_COUNTS, &level.counts, counts_pos)?;
    if has_ycbcr_subsampling {
        entry_u16_array_inline(
            &mut out,
            TAG_YCBCR_SUBSAMPLING,
            &[ycbcr_subsampling.0, ycbcr_subsampling.1],
        );
    }
    push_u64(&mut out, next);
    Ok(out)
}

fn directory_entry_count(codec: Codec, color: ColorModel) -> usize {
    11 + usize::from(codec == Codec::Jpeg && color == ColorModel::YCbCr)
}

fn entry_header(out: &mut Vec<u8>, code: u16, ty: u16, count: u64) {
    push_u16(out, code);
    push_u16(out, ty);
    push_u64(out, count);
}

fn entry_u16(out: &mut Vec<u8>, code: u16, value: u16) {
    entry_header(out, code, TYPE_U16, 1);
    push_u16(out, value);
    out.extend_from_slice(&[0; 6]);
}

fn entry_u16_array_inline(out: &mut Vec<u8>, code: u16, values: &[u16]) {
    entry_header(out, code, TYPE_U16, u64::try_from(values.len()).unwrap());
    for value in values {
        push_u16(out, *value);
    }
    for _ in values.len()..4 {
        push_u16(out, 0);
    }
}

fn entry_u32(out: &mut Vec<u8>, code: u16, value: u32) {
    entry_header(out, code, TYPE_U32, 1);
    push_u32(out, value);
    out.extend_from_slice(&[0; 4]);
}

fn entry_u64_array(out: &mut Vec<u8>, code: u16, values: &[u64], pos: Option<u64>) -> Result<()> {
    entry_header(
        out,
        code,
        TYPE_U64,
        u64::try_from(values.len()).map_err(|_| Error::Unsupported("too many values"))?,
    );
    if values.len() == 1 {
        push_u64(out, values[0]);
    } else {
        push_u64(
            out,
            pos.ok_or(Error::InvalidInput("missing array position"))?,
        );
    }
    Ok(())
}

fn entry_u32_array(out: &mut Vec<u8>, code: u16, values: &[u32], pos: Option<u64>) -> Result<()> {
    entry_header(
        out,
        code,
        TYPE_U32,
        u64::try_from(values.len()).map_err(|_| Error::Unsupported("too many values"))?,
    );
    if values.len() <= 2 {
        for v in values {
            push_u32(out, *v);
        }
        for _ in values.len()..2 {
            push_u32(out, 0);
        }
    } else {
        push_u64(
            out,
            pos.ok_or(Error::InvalidInput("missing array position"))?,
        );
    }
    Ok(())
}
