use zif_tiff::{Codec, ColorModel, DataChunk, Image, ParseState, Parser, WriteBatch, Writer};

fn apply(file: &mut Vec<u8>, batch: WriteBatch) {
    for action in batch.into_actions() {
        let offset = usize::try_from(action.offset).unwrap();
        let end = offset + action.bytes.len();
        if file.len() < end {
            file.resize(end, 0);
        }
        file[offset..end].copy_from_slice(&action.bytes);
    }
}

fn read_all(file: &[u8]) -> Image {
    let mut parser = Parser::new();
    let state = parser
        .feed(DataChunk::from_start(0, file.to_vec()).unwrap())
        .unwrap();
    assert!(matches!(state, ParseState::Done { .. }));
    parser.image().unwrap().clone()
}

#[test]
fn writer_roundtrips_one_tile_file() {
    let mut writer = Writer::new()
        .dimensions((16, 16))
        .tile_size((16, 16))
        .unwrap()
        .codec(Codec::Jpeg)
        .color_model(ColorModel::YCbCr)
        .channels(3)
        .unwrap()
        .build()
        .unwrap();
    let mut file = Vec::new();
    apply(&mut file, writer.put_tile((0, 0), b"jpeg").unwrap());

    let image = read_all(&file);
    assert_eq!(image.dimensions(), (16, 16));
    assert_eq!(image.level_count(), 1);
    let tile = image.level_tiles(0).unwrap().next().unwrap();
    let bytes = tile.byte_range();
    let start = usize::try_from(bytes.start).unwrap();
    let end = usize::try_from(bytes.end).unwrap();
    assert_eq!(&file[start..end], b"jpeg");
}

#[test]
fn writer_roundtrips_multi_tile_file_and_crops() {
    let mut writer = Writer::new()
        .dimensions((40, 40))
        .tile_size((16, 16))
        .unwrap()
        .codec(Codec::Png)
        .color_model(ColorModel::Rgb)
        .channels(3)
        .unwrap()
        .build()
        .unwrap();
    let mut file = Vec::new();
    apply(&mut file, writer.put_tile((0, 0), b"a").unwrap());
    apply(&mut file, writer.put_tile((1, 0), b"bb").unwrap());
    apply(&mut file, writer.put_tile((2, 2), b"ccc").unwrap());

    let image = read_all(&file);
    let level = image.level(0).unwrap();
    assert_eq!(level.tile_grid(), (3, 3));
    let tile_count = image
        .viewport_tiles(0, (15..40, 1..40))
        .unwrap()
        .count();
    assert_eq!(tile_count, 9);
    let edge = image.level(0).unwrap().tile(2, 2).unwrap();
    assert_eq!(edge.position(), (32, 32));
    assert_eq!(edge.size(), (8, 8));
}

#[test]
fn reader_accepts_non_exact_chunk() {
    let mut writer = Writer::new()
        .dimensions((16, 16))
        .tile_size((16, 16))
        .unwrap()
        .codec(Codec::Jpeg)
        .color_model(ColorModel::YCbCr)
        .channels(3)
        .unwrap()
        .build()
        .unwrap();
    let mut file = Vec::new();
    apply(&mut file, writer.put_tile((0, 0), b"jpeg").unwrap());

    let mut parser = Parser::new();
    assert!(matches!(
        parser.feed(DataChunk::default()).unwrap(),
        ParseState::Need { .. }
    ));
    assert!(matches!(
        parser
            .feed(DataChunk::from_start(0, file).unwrap())
            .unwrap(),
        ParseState::Done { .. }
    ));
    assert_eq!(parser.image().unwrap().dimensions(), (16, 16));
}

#[test]
fn chunk_rejects_incoherent_range() {
    assert!(DataChunk::new(0..4, vec![1, 2, 3]).is_err());
}
