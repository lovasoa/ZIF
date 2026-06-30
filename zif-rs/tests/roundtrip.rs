use zif::{Chunk, Codec, ColorModel, ReadStatus, Reader, WriteBatch, WriteOp, Writer};

fn apply(file: &mut Vec<u8>, batch: WriteBatch) {
    for op in batch.into_ops() {
        match op {
            WriteOp::InitHeader(bytes) => {
                if file.len() < 16 {
                    file.resize(16, 0);
                }
                file[..16].copy_from_slice(&bytes);
            }
            WriteOp::Append(bytes) => file.extend_from_slice(&bytes),
            WriteOp::PatchU64 { offset, value } => {
                let offset = usize::try_from(offset.get()).unwrap();
                file[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
            }
        }
    }
}

fn read_all(file: &[u8]) -> zif::Zif {
    let mut reader = Reader::new();
    let status = reader
        .advance(Chunk::from_start(0, file.to_vec()).unwrap())
        .unwrap();
    assert!(matches!(status, ReadStatus::Done { .. }));
    reader.zif().unwrap().clone()
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

    let zif = read_all(&file);
    assert_eq!(zif.dimensions(), (16, 16));
    assert_eq!(zif.level_count(), 1);
    let tile = zif.get_level_tiles(0).unwrap().next().unwrap();
    assert_eq!(tile.bytes(), 16..20);
    let bytes = tile.bytes();
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

    let zif = read_all(&file);
    let level = zif.level(0).unwrap();
    assert_eq!(level.tile_grid(), (3, 3));
    let tile_count = zif
        .get_cropped_level_tiles(0, (15..40, 1..40))
        .unwrap()
        .count();
    assert_eq!(tile_count, 9);
    let edge = zif.level(0).unwrap().tile(2, 2).unwrap();
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

    let mut reader = Reader::new();
    assert!(matches!(
        reader.advance(Chunk::default()).unwrap(),
        ReadStatus::Need { .. }
    ));
    assert!(matches!(
        reader.advance(Chunk::from_start(0, file).unwrap()).unwrap(),
        ReadStatus::Done { .. }
    ));
    assert_eq!(reader.zif().unwrap().dimensions(), (16, 16));
}

#[test]
fn chunk_rejects_incoherent_range() {
    assert!(Chunk::new(0..4, vec![1, 2, 3]).is_err());
}
