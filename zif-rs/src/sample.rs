use alloc::vec::Vec;

use crate::{Chunk, Codec, ColorModel, ReadStatus, Reader, WriteBatch, Writer, Zif};

/// Returns a small valid ZIF file for examples and tests.
pub fn zif_bytes() -> Vec<u8> {
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

/// Returns a small valid ZIF file for examples and tests.
pub fn file() -> Vec<u8> {
    zif_bytes()
}

/// Returns parsed metadata for [`file`].
pub fn zif() -> Zif {
    let file = zif_bytes();
    let mut reader = Reader::new();
    let status = reader
        .advance(Chunk::from_start(0, file).expect("coherent chunk"))
        .expect("sample parses");
    assert!(matches!(status, ReadStatus::Done { .. }));
    reader.into_zif().expect("reader is done")
}

fn apply(file: &mut Vec<u8>, batch: WriteBatch) {
    for op in batch.into_ops() {
        let offset = usize::try_from(op.offset).expect("sample offsets fit usize");
        let end = offset + op.bytes.len();
        if file.len() < end {
            file.resize(end, 0);
        }
        file[offset..end].copy_from_slice(&op.bytes);
    }
}
