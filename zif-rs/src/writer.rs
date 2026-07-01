use alloc::vec::Vec;

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
    pub fn new(dimensions: (u64, u64), tile_size: (u32, u32)) -> Result<Self> {
        validate_level_spec(dimensions, tile_size)?;
        Ok(Self {
            dimensions,
            tile_size,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PyramidMode {
    SingleLevel,
    ToSingleTile,
    To1x1,
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
    pyramid: PyramidMode,
}

impl WriterBuilder {
    pub fn dimensions(mut self, dimensions: (u64, u64)) -> Self {
        self.dimensions = Some(dimensions);
        self
    }
    pub fn tile_size(mut self, tile_size: (u32, u32)) -> Result<Self> {
        self.tile_size = Some(tile_size);
        Ok(self)
    }
    pub fn level(mut self, level: LevelSpec) -> Self {
        self.levels.push(level);
        self
    }
    pub fn codec(mut self, codec: Codec) -> Self {
        self.codec = Some(codec);
        self
    }
    pub fn color_model(mut self, color_model: ColorModel) -> Self {
        self.color_model = Some(color_model);
        self
    }
    pub fn channels(mut self, channels: u16) -> Result<Self> {
        validate_channels(channels)?;
        self.channels = Some(channels);
        Ok(self)
    }
    pub fn ycbcr_subsampling(mut self, subsampling: (u16, u16)) -> Result<Self> {
        validate_ycbcr_subsampling(subsampling)?;
        self.ycbcr_subsampling = Some(subsampling);
        Ok(self)
    }
    pub fn preserve_nonstandard_ycbcr_subsampling(
        mut self,
        subsampling: (u16, u16),
    ) -> Result<Self> {
        validate_nonstandard_ycbcr_subsampling(subsampling)?;
        self.ycbcr_subsampling = Some(subsampling);
        Ok(self)
    }
    pub fn pyramid(mut self) -> Self {
        self.pyramid = PyramidMode::ToSingleTile;
        self
    }
    pub fn pyramid_to_1x1(mut self) -> Self {
        self.pyramid = PyramidMode::To1x1;
        self
    }
    pub fn build(mut self) -> Result<Writer> {
        if self.levels.is_empty() {
            let dimensions = self
                .dimensions
                .ok_or(Error::InvalidInput("missing dimensions"))?;
            let tile_size = self
                .tile_size
                .ok_or(Error::InvalidInput("missing tile size"))?;
            let mut dims = dimensions;
            self.levels.push(LevelSpec::new(dims, tile_size)?);
            match self.pyramid {
                PyramidMode::SingleLevel => {}
                PyramidMode::ToSingleTile => {
                    while dims.0 > u64::from(tile_size.0) || dims.1 > u64::from(tile_size.1) {
                        dims = (dims.0.div_ceil(2), dims.1.div_ceil(2));
                        self.levels.push(LevelSpec::new(dims, tile_size)?);
                    }
                }
                PyramidMode::To1x1 => {
                    while dims.0 > 1 || dims.1 > 1 {
                        dims = (dims.0.div_ceil(2), dims.1.div_ceil(2));
                        self.levels.push(LevelSpec::new(dims, tile_size)?);
                    }
                }
            }
        }
        let codec = self.codec.ok_or(Error::InvalidInput("missing codec"))?;
        let color_model = self
            .color_model
            .ok_or(Error::InvalidInput("missing color model"))?;
        let channels = self
            .channels
            .ok_or(Error::InvalidInput("missing channels"))?;
        validate_color_channels(color_model, channels)?;
        let ycbcr_subsampling = self.ycbcr_subsampling.unwrap_or((2, 2));
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
                dir_offset: 0,
                offsets_base: 0,
                counts_base: 0,
                offsets: alloc::vec![0; len],
                counts: alloc::vec![0; len],
            });
        }
        let mut writer = Writer {
            levels,
            codec,
            color_model,
            channels,
            ycbcr_subsampling,
            file_len: 0,
            initialized: false,
        };
        writer.recompute_layout();
        Ok(writer)
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
    pub fn new() -> WriterBuilder {
        WriterBuilder {
            dimensions: None,
            tile_size: None,
            levels: Vec::new(),
            codec: None,
            color_model: None,
            channels: None,
            ycbcr_subsampling: None,
            pyramid: PyramidMode::SingleLevel,
        }
    }

    pub fn init(&mut self) -> Result<WriteBatch> {
        if self.initialized {
            return Err(Error::InvalidInput("already initialized"));
        }
        let mut batch = WriteBatch { ops: Vec::new() };
        self.emit_init(&mut batch)?;
        Ok(batch)
    }

    pub fn put_tile(&mut self, coord: (u64, u64), bytes: impl AsRef<[u8]>) -> Result<WriteBatch> {
        self.put_tile_at_level(0, coord, bytes)
    }

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
        let state = self
            .levels
            .get(level)
            .ok_or(Error::InvalidInput("level index out of range"))?;
        let (col, row) = coord;
        if col >= state.tiles_across || row >= state.tiles_down {
            return Err(Error::InvalidInput("tile coordinate out of range"));
        }
        let index = usize::try_from(row * state.tiles_across + col)
            .map_err(|_| Error::InvalidInput("tile index too large"))?;

        let mut batch = WriteBatch { ops: Vec::new() };
        if !self.initialized {
            self.emit_init(&mut batch)?;
        }

        let tile_offset = self.reserve(bytes.len())?;
        batch.ops.push(WriteOp {
            offset: tile_offset,
            bytes: bytes.to_vec(),
        });

        let state = &self.levels[level];
        batch.ops.push(WriteOp {
            offset: state.offsets_base + index as u64 * 8,
            bytes: tile_offset.to_le_bytes().to_vec(),
        });
        batch.ops.push(WriteOp {
            offset: state.counts_base + index as u64 * 4,
            bytes: u32::try_from(bytes.len())
                .map_err(|_| Error::InvalidInput("tile payload exceeds u32::MAX"))?
                .to_le_bytes()
                .to_vec(),
        });

        let state = &mut self.levels[level];
        state.offsets[index] = tile_offset;
        state.counts[index] = u32::try_from(bytes.len())
            .map_err(|_| Error::InvalidInput("tile payload exceeds u32::MAX"))?;

        Ok(batch)
    }

    pub fn set_dimensions(&mut self, level: usize, dimensions: (u64, u64)) -> Result<WriteBatch> {
        if level >= self.levels.len() {
            return Err(Error::InvalidInput("level index out of range"));
        }
        let tile_size = self.levels[level].spec.tile_size;
        validate_level_spec(dimensions, tile_size)?;
        let (across, down, count) = tile_count(
            dimensions.0,
            dimensions.1,
            u64::from(tile_size.0),
            u64::from(tile_size.1),
        )?;
        let len = usize::try_from(count)
            .map_err(|_| Error::Unsupported("too many tiles for in-memory writer"))?;

        let mut batch = WriteBatch { ops: Vec::new() };

        if !self.initialized {
            let spec = LevelSpec::new(dimensions, tile_size)?;
            self.levels[level] = LevelState {
                spec,
                tiles_across: across,
                tiles_down: down,
                dir_offset: self.levels[level].dir_offset,
                offsets_base: self.levels[level].offsets_base,
                counts_base: self.levels[level].counts_base,
                offsets: alloc::vec![0; len],
                counts: alloc::vec![0; len],
            };
            self.recompute_layout();
            return Ok(batch);
        }

        let old = &self.levels[level];
        let dir_offset = old.dir_offset;

        let mut new_offsets = alloc::vec![0u64; len];
        let mut new_counts = alloc::vec![0u32; len];

        let copy_rows = old.tiles_down.min(down);
        let copy_cols = old.tiles_across.min(across);
        for row in 0..copy_rows {
            for col in 0..copy_cols {
                let old_i = usize::try_from(row * old.tiles_across + col)
                    .map_err(|_| Error::InvalidInput("tile index too large"))?;
                let new_i = usize::try_from(row * across + col)
                    .map_err(|_| Error::InvalidInput("tile index too large"))?;
                new_offsets[new_i] = old.offsets[old_i];
                new_counts[new_i] = old.counts[old_i];
            }
        }

        let new_offsets_base = if len > 1 {
            let base = self.reserve(len * 8)?;
            let mut buf = Vec::with_capacity(len * 8);
            for v in &new_offsets {
                push_u64(&mut buf, *v);
            }
            batch.ops.push(WriteOp {
                offset: base,
                bytes: buf,
            });
            batch.ops.push(WriteOp {
                offset: dir_offset + 8 + 9 * ENTRY_LEN as u64 + 12,
                bytes: base.to_le_bytes().to_vec(),
            });
            base
        } else {
            batch.ops.push(WriteOp {
                offset: dir_offset + 8 + 9 * ENTRY_LEN as u64 + 12,
                bytes: new_offsets[0].to_le_bytes().to_vec(),
            });
            dir_offset + 8 + 9 * ENTRY_LEN as u64 + 12
        };

        let new_counts_base = if len > 2 {
            let base = self.reserve(len * 4)?;
            let mut buf = Vec::with_capacity(len * 4);
            for v in &new_counts {
                push_u32(&mut buf, *v);
            }
            batch.ops.push(WriteOp {
                offset: base,
                bytes: buf,
            });
            batch.ops.push(WriteOp {
                offset: dir_offset + 8 + 10 * ENTRY_LEN as u64 + 12,
                bytes: base.to_le_bytes().to_vec(),
            });
            base
        } else {
            let slot = dir_offset + 8 + 10 * ENTRY_LEN as u64 + 12;
            for (i, &v) in new_counts.iter().enumerate() {
                batch.ops.push(WriteOp {
                    offset: slot + i as u64 * 4,
                    bytes: v.to_le_bytes().to_vec(),
                });
            }
            slot
        };

        batch.ops.push(WriteOp {
            offset: dir_offset + 8 + 12,
            bytes: u32::try_from(dimensions.0)
                .map_err(|_| Error::Unsupported("width exceeds u32"))?
                .to_le_bytes()
                .to_vec(),
        });
        batch.ops.push(WriteOp {
            offset: dir_offset + 8 + ENTRY_LEN as u64 + 12,
            bytes: u32::try_from(dimensions.1)
                .map_err(|_| Error::Unsupported("height exceeds u32"))?
                .to_le_bytes()
                .to_vec(),
        });
        batch.ops.push(WriteOp {
            offset: dir_offset + 8 + 9 * ENTRY_LEN as u64 + 4,
            bytes: count.to_le_bytes().to_vec(),
        });
        batch.ops.push(WriteOp {
            offset: dir_offset + 8 + 10 * ENTRY_LEN as u64 + 4,
            bytes: count.to_le_bytes().to_vec(),
        });

        let spec = LevelSpec::new(dimensions, tile_size)?;
        self.levels[level] = LevelState {
            spec,
            tiles_across: across,
            tiles_down: down,
            dir_offset,
            offsets_base: new_offsets_base,
            counts_base: new_counts_base,
            offsets: new_offsets,
            counts: new_counts,
        };

        Ok(batch)
    }

    pub fn add_level(&mut self, index: usize, spec: LevelSpec) -> Result<WriteBatch> {
        if index > self.levels.len() {
            return Err(Error::InvalidInput("insertion index out of range"));
        }
        validate_level_spec(spec.dimensions, spec.tile_size)?;
        let (across, down, count) = tile_count(
            spec.dimensions.0,
            spec.dimensions.1,
            u64::from(spec.tile_size.0),
            u64::from(spec.tile_size.1),
        )?;
        let len = usize::try_from(count)
            .map_err(|_| Error::Unsupported("too many tiles for in-memory writer"))?;

        let mut batch = WriteBatch { ops: Vec::new() };

        if !self.initialized {
            self.levels.insert(
                index,
                LevelState {
                    spec,
                    tiles_across: across,
                    tiles_down: down,
                    dir_offset: 0,
                    offsets_base: 0,
                    counts_base: 0,
                    offsets: alloc::vec![0; len],
                    counts: alloc::vec![0; len],
                },
            );
            self.recompute_layout();
            return Ok(batch);
        }

        let entry_count = self.entry_count() as u64;
        let dir_size = 8 + entry_count * ENTRY_LEN as u64 + 8;
        let offsets_size = if len > 1 { len as u64 * 8 } else { 0 };
        let counts_size = if len > 2 { len as u64 * 4 } else { 0 };

        let new_dir_offset = self.file_len;
        let next = if index < self.levels.len() {
            self.levels[index].dir_offset
        } else {
            0
        };

        let offsets_base = if len > 1 {
            new_dir_offset + dir_size
        } else {
            new_dir_offset + 8 + 9 * ENTRY_LEN as u64 + 12
        };
        let counts_base = if len > 2 {
            new_dir_offset + dir_size + offsets_size
        } else {
            new_dir_offset + 8 + 10 * ENTRY_LEN as u64 + 12
        };

        let offsets_pos = if len > 1 { Some(offsets_base) } else { None };
        let counts_pos = if len > 2 { Some(counts_base) } else { None };

        let temp = LevelState {
            spec,
            tiles_across: across,
            tiles_down: down,
            dir_offset: new_dir_offset,
            offsets_base,
            counts_base,
            offsets: alloc::vec![0; len],
            counts: alloc::vec![0; len],
        };

        let dir = encode_directory(
            &temp,
            offsets_pos,
            counts_pos,
            next,
            self.codec,
            self.color_model,
            self.channels,
            self.ycbcr_subsampling,
        )?;

        debug_assert_eq!(dir.len() as u64, dir_size);

        let total_size = usize::try_from(dir_size)
            .map_err(|_| Error::Unsupported("dir size exceeds address space"))?
            + usize::try_from(offsets_size)
                .map_err(|_| Error::Unsupported("offsets size exceeds address space"))?
            + usize::try_from(counts_size)
                .map_err(|_| Error::Unsupported("counts size exceeds address space"))?;
        let mut payload = Vec::with_capacity(total_size);
        payload.extend_from_slice(&dir);
        payload.resize(total_size, 0);

        batch.ops.push(WriteOp {
            offset: new_dir_offset,
            bytes: payload,
        });
        self.file_len += total_size as u64;

        if index == 0 {
            batch.ops.push(WriteOp {
                offset: 8,
                bytes: new_dir_offset.to_le_bytes().to_vec(),
            });
        } else {
            let prev_dir = self.levels[index - 1].dir_offset;
            let next_ptr = prev_dir + 8 + entry_count * ENTRY_LEN as u64;
            batch.ops.push(WriteOp {
                offset: next_ptr,
                bytes: new_dir_offset.to_le_bytes().to_vec(),
            });
        }

        self.levels.insert(
            index,
            LevelState {
                spec: temp.spec,
                tiles_across: temp.tiles_across,
                tiles_down: temp.tiles_down,
                dir_offset: new_dir_offset,
                offsets_base,
                counts_base,
                offsets: alloc::vec![0; len],
                counts: alloc::vec![0; len],
            },
        );

        Ok(batch)
    }

    pub fn remove_level(&mut self, index: usize) -> Result<WriteBatch> {
        if self.levels.len() <= 1 {
            return Err(Error::InvalidInput("cannot remove the only level"));
        }
        if index >= self.levels.len() {
            return Err(Error::InvalidInput("level index out of range"));
        }

        let mut batch = WriteBatch { ops: Vec::new() };

        if !self.initialized {
            self.levels.remove(index);
            self.recompute_layout();
            return Ok(batch);
        }

        let entry_count = self.entry_count() as u64;
        let next = if index + 1 < self.levels.len() {
            self.levels[index + 1].dir_offset
        } else {
            0
        };

        if index == 0 {
            batch.ops.push(WriteOp {
                offset: 8,
                bytes: next.to_le_bytes().to_vec(),
            });
        } else {
            let prev_dir = self.levels[index - 1].dir_offset;
            let next_ptr = prev_dir + 8 + entry_count * ENTRY_LEN as u64;
            batch.ops.push(WriteOp {
                offset: next_ptr,
                bytes: next.to_le_bytes().to_vec(),
            });
        }

        self.levels.remove(index);
        Ok(batch)
    }

    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    pub fn level_dimensions(&self, level: usize) -> Result<(u64, u64)> {
        self.levels
            .get(level)
            .map(|s| s.spec.dimensions)
            .ok_or(Error::InvalidInput("level index out of range"))
    }

    pub fn level_tile_size(&self, level: usize) -> Result<(u32, u32)> {
        self.levels
            .get(level)
            .map(|s| s.spec.tile_size)
            .ok_or(Error::InvalidInput("level index out of range"))
    }

    pub fn level_tile_grid(&self, level: usize) -> Result<(u64, u64)> {
        self.levels
            .get(level)
            .map(|s| (s.tiles_across, s.tiles_down))
            .ok_or(Error::InvalidInput("level index out of range"))
    }

    fn recompute_layout(&mut self) {
        if self.initialized {
            return;
        }
        let entry_count = self.entry_count() as u64;
        let dir_size = 8 + entry_count * ENTRY_LEN as u64 + 8;

        let mut cursor = 16u64;

        for level in &mut self.levels {
            level.dir_offset = cursor;
            cursor += dir_size;
        }

        for level in &mut self.levels {
            if level.offsets.len() > 1 {
                level.offsets_base = cursor;
                cursor += level.offsets.len() as u64 * 8;
            } else {
                level.offsets_base = level.dir_offset + 8 + 9 * ENTRY_LEN as u64 + 12;
            }
        }

        for level in &mut self.levels {
            if level.counts.len() > 2 {
                level.counts_base = cursor;
                cursor += level.counts.len() as u64 * 4;
            } else {
                level.counts_base = level.dir_offset + 8 + 10 * ENTRY_LEN as u64 + 12;
            }
        }

        self.file_len = cursor;
    }

    fn emit_init(&mut self, batch: &mut WriteBatch) -> Result<()> {
        let entry_count = self.entry_count() as u64;
        let dir_size = 8 + entry_count * ENTRY_LEN as u64 + 8;

        let mut header = [0u8; 16];
        header[..8].copy_from_slice(&[0x49, 0x49, 0x2b, 0x00, 0x08, 0x00, 0x00, 0x00]);
        header[8..16].copy_from_slice(&16u64.to_le_bytes());
        batch.ops.push(WriteOp {
            offset: 0,
            bytes: header.to_vec(),
        });

        let mut dirs = Vec::new();
        for (i, level) in self.levels.iter().enumerate() {
            let next = if i + 1 < self.levels.len() {
                self.levels[i + 1].dir_offset
            } else {
                0
            };
            let offsets_pos = if level.offsets.len() > 1 {
                Some(level.offsets_base)
            } else {
                None
            };
            let counts_pos = if level.counts.len() > 2 {
                Some(level.counts_base)
            } else {
                None
            };
            let dir = encode_directory(
                level,
                offsets_pos,
                counts_pos,
                next,
                self.codec,
                self.color_model,
                self.channels,
                self.ycbcr_subsampling,
            )?;
            dirs.extend_from_slice(&dir);
        }
        batch.ops.push(WriteOp {
            offset: 16,
            bytes: dirs,
        });

        let mut arrays = Vec::new();
        for level in &self.levels {
            if level.offsets.len() > 1 {
                arrays.resize(arrays.len() + level.offsets.len() * 8, 0);
            }
        }
        for level in &self.levels {
            if level.counts.len() > 2 {
                arrays.resize(arrays.len() + level.counts.len() * 4, 0);
            }
        }
        if !arrays.is_empty() {
            let arrays_start = 16 + (self.levels.len() as u64 * dir_size);
            batch.ops.push(WriteOp {
                offset: arrays_start,
                bytes: arrays,
            });
        }

        self.initialized = true;
        Ok(())
    }

    fn reserve(&mut self, len: usize) -> Result<u64> {
        let offset = self.file_len;
        self.file_len = self
            .file_len
            .checked_add(u64::try_from(len).map_err(|_| Error::InvalidInput("length too large"))?)
            .ok_or(Error::InvalidInput("file length overflow"))?;
        Ok(offset)
    }

    fn entry_count(&self) -> usize {
        11 + usize::from(self.codec == Codec::Jpeg && self.color_model == ColorModel::YCbCr)
    }
}

#[derive(Debug, Clone)]
struct LevelState {
    spec: LevelSpec,
    tiles_across: u64,
    tiles_down: u64,
    dir_offset: u64,
    offsets_base: u64,
    counts_base: u64,
    offsets: Vec<u64>,
    counts: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOp {
    pub offset: u64,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteBatch {
    ops: Vec<WriteOp>,
}

impl WriteBatch {
    pub fn ops(&self) -> &[WriteOp] {
        &self.ops
    }
    pub fn into_ops(self) -> Vec<WriteOp> {
        self.ops
    }
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
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
