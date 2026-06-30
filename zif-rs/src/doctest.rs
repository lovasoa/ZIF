//! Helpers used by rustdoc examples.

use alloc::vec::Vec;

use crate::{Chunk, Codec, ColorModel, ReadStatus, Reader, WriteBatch, WriteOp, Writer, Zif};

pub fn sample_file() -> Vec<u8> {
    let mut writer = Writer::new()
        .dimensions((40, 40))
        .tile_size((16, 16))
        .expect("valid tile size")
        .codec(Codec::Jpeg)
        .color_model(ColorModel::YCbCr)
        .channels(3)
        .expect("valid channels")
        .build()
        .expect("valid writer");

    let mut file = Vec::new();
    apply(
        &mut file,
        writer.put_tile((0, 0), b"tile-0").expect("tile writes"),
    );
    apply(
        &mut file,
        writer.put_tile((1, 0), b"tile-1").expect("tile writes"),
    );
    apply(
        &mut file,
        writer.put_tile((2, 2), b"tile-8").expect("tile writes"),
    );
    file
}

pub fn sample_zif() -> Zif {
    let file = sample_file();
    let mut reader = Reader::new();
    let status = reader
        .advance(Chunk::from_start(0, file).expect("coherent chunk"))
        .expect("sample parses");
    assert_eq!(status, ReadStatus::Done);
    reader.zif().expect("reader is done").clone()
}

pub fn apply(file: &mut Vec<u8>, batch: WriteBatch) {
    for op in batch.into_ops() {
        match op {
            WriteOp::InitHeader(bytes) => {
                if file.len() < bytes.len() {
                    file.resize(bytes.len(), 0);
                }
                file[..bytes.len()].copy_from_slice(&bytes);
            }
            WriteOp::Append(bytes) => file.extend_from_slice(&bytes),
            WriteOp::PatchU64 { offset, value } => {
                let offset = usize::try_from(offset.get()).expect("sample offsets fit usize");
                file[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
            }
        }
    }
}
