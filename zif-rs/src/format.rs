use alloc::vec::Vec;

use crate::{Error, Result};

pub const ENTRY_LEN: usize = 20;

pub const TAG_WIDTH: u16 = 256;
pub const TAG_HEIGHT: u16 = 257;
pub const TAG_BITS: u16 = 258;
pub const TAG_CODEC: u16 = 259;
pub const TAG_COLOR: u16 = 262;
pub const TAG_CHANNELS: u16 = 277;
pub const TAG_INTERLEAVE: u16 = 284;
pub const TAG_TILE_WIDTH: u16 = 322;
pub const TAG_TILE_HEIGHT: u16 = 323;
pub const TAG_TILE_OFFSETS: u16 = 324;
pub const TAG_TILE_COUNTS: u16 = 325;

pub const TYPE_U16: u16 = 3;
pub const TYPE_U32: u16 = 4;
pub const TYPE_U64: u16 = 16;

pub fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let s = bytes
        .get(offset..offset + 2)
        .ok_or(Error::MalformedFile("unexpected end of data"))?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

pub fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let s = bytes
        .get(offset..offset + 4)
        .ok_or(Error::MalformedFile("unexpected end of data"))?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

pub fn read_u64(bytes: &[u8], offset: usize) -> Result<u64> {
    let s = bytes
        .get(offset..offset + 8)
        .ok_or(Error::MalformedFile("unexpected end of data"))?;
    Ok(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

pub fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub fn checked_len(range_start: u64, len: usize) -> Result<core::ops::Range<u64>> {
    let len = u64::try_from(len).map_err(|_| Error::InvalidInput("length does not fit u64"))?;
    let end = range_start
        .checked_add(len)
        .ok_or(Error::InvalidInput("range end overflows u64"))?;
    Ok(range_start..end)
}

pub fn ceil_div(a: u64, b: u64) -> Result<u64> {
    if b == 0 {
        return Err(Error::InvalidInput("division by zero"));
    }
    Ok(a / b + u64::from(a % b != 0))
}

pub fn tile_count(
    width: u64,
    height: u64,
    tile_width: u64,
    tile_height: u64,
) -> Result<(u64, u64, u64)> {
    let across = ceil_div(width, tile_width)?;
    let down = ceil_div(height, tile_height)?;
    let count = across
        .checked_mul(down)
        .ok_or(Error::MalformedFile("tile count overflows u64"))?;
    Ok((across, down, count))
}
