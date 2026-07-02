use alloc::vec::Vec;

use crate::{Codec, ColorModel, DataChunk, Image, ParseState, Parser, WriteBatch, Writer};

/// Returns a small valid ZIF file for examples and tests.
pub fn file() -> Vec<u8> {
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

/// Returns parsed metadata for [`file()`].
pub fn image() -> Image {
    let file = file();
    let mut parser = Parser::new();
    let state = parser
        .feed(DataChunk::from_start(0, file).expect("coherent chunk"))
        .expect("sample parses");
    assert!(matches!(state, ParseState::Done { .. }));
    parser.finish().expect("parser is done")
}

fn apply(file: &mut Vec<u8>, batch: WriteBatch) {
    for action in batch.into_actions() {
        let offset = usize::try_from(action.offset).expect("sample offsets fit usize");
        let end = offset + action.bytes.len();
        if file.len() < end {
            file.resize(end, 0);
        }
        file[offset..end].copy_from_slice(&action.bytes);
    }
}
