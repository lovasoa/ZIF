use zif::{
    ChainKind, Chunk, Codec, ColorModel, Error, LevelSpec, ReadStatus, Reader, Request, WriteBatch,
    Writer,
};

fn apply(file: &mut Vec<u8>, batch: WriteBatch) {
    for op in batch.into_ops() {
        let offset = usize::try_from(op.offset).unwrap();
        let end = offset + op.bytes.len();
        if file.len() < end {
            file.resize(end, 0);
        }
        file[offset..end].copy_from_slice(&op.bytes);
    }
}

fn reader_file(
    dimensions: (u64, u64),
    tile_size: (u32, u32),
    tiles: &[((u64, u64), &[u8])],
) -> Vec<u8> {
    let mut writer = Writer::new()
        .dimensions(dimensions)
        .tile_size(tile_size)
        .unwrap()
        .codec(Codec::Jpeg)
        .color_model(ColorModel::YCbCr)
        .channels(3)
        .unwrap()
        .build()
        .unwrap();
    let mut file = Vec::new();
    for &(coord, bytes) in tiles {
        apply(&mut file, writer.put_tile(coord, bytes).unwrap());
    }
    file
}

fn parse(file: &[u8]) -> zif::Zif {
    let mut reader = Reader::new();
    assert!(matches!(
        reader
            .advance(Chunk::from_start(0, file.to_vec()).unwrap())
            .unwrap(),
        ReadStatus::Done { .. }
    ));
    reader.zif().unwrap().clone()
}

#[derive(Debug)]
struct RawEntry {
    code: u16,
    ty: u16,
    count: u64,
    slot: [u8; 8],
}

fn assert_raw_directory_chain(file: &[u8], expected_levels: usize, expected_entries: usize) {
    let mut dir = read_u64(file, 8);
    for _ in 0..expected_levels {
        assert_ne!(dir, 0);
        let dir_offset = usize::try_from(dir).unwrap();
        let count = usize::try_from(read_u64(file, dir_offset)).unwrap();
        assert_eq!(count, expected_entries);
        let entries = raw_entries(file, dir_offset, count);
        let codes: Vec<_> = entries.iter().map(|entry| entry.code).collect();
        assert!(codes.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(entry(&entries, 324).ty, 16);
        assert_eq!(entry(&entries, 325).ty, 4);
        if expected_entries == 12 {
            assert_eq!(entry(&entries, 530).ty, 3);
            assert_eq!(entry(&entries, 530).count, 2);
        }
        let next_pos = dir_offset + 8 + count * 20;
        assert!(next_pos + 8 <= file.len());
        dir = read_u64(file, next_pos);
    }
    assert_eq!(dir, 0);
}

fn raw_entries(file: &[u8], dir: usize, count: usize) -> Vec<RawEntry> {
    (0..count)
        .map(|index| {
            let offset = dir + 8 + index * 20;
            let mut slot = [0; 8];
            slot.copy_from_slice(&file[offset + 12..offset + 20]);
            RawEntry {
                code: read_u16(file, offset),
                ty: read_u16(file, offset + 2),
                count: read_u64(file, offset + 4),
                slot,
            }
        })
        .collect()
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn first_directory_entries(file: &[u8]) -> Vec<RawEntry> {
    let dir = usize::try_from(read_u64(file, 8)).unwrap();
    let count = usize::try_from(read_u64(file, dir)).unwrap();
    raw_entries(file, dir, count)
}

fn entry(entries: &[RawEntry], code: u16) -> &RawEntry {
    entries.iter().find(|entry| entry.code == code).unwrap()
}

#[test]
fn request_accessors_and_validation() {
    let req = Request::new(5..12).unwrap();
    assert_eq!(req.start(), 5);
    assert_eq!(req.end(), 12);
    assert_eq!(req.len(), 7);
    assert_eq!(req.range(), 5..12);
    assert!(!req.is_empty());
    let (start, end) = (12, 5);
    assert!(Request::new(start..end).is_err());
    assert!(Request::new(5..5).unwrap().is_empty());
}

#[test]
fn chunk_accessors_and_validation() {
    let chunk = Chunk::from_start(9, vec![1, 2, 3]).unwrap();
    assert_eq!(chunk.start(), 9);
    assert_eq!(chunk.end(), 12);
    assert_eq!(chunk.range(), 9..12);
    assert_eq!(chunk.bytes(), &[1, 2, 3]);
    let (start, end) = (10, 9);
    assert!(Chunk::new(start..end, Vec::<u8>::new()).is_err());
    assert!(Chunk::new(0..2, vec![1]).is_err());
}

#[test]
fn reader_rejects_bad_header() {
    let mut file = reader_file((16, 16), (16, 16), &[((0, 0), b"tile")]);
    file[0] = 0;
    let mut reader = Reader::new();
    assert!(matches!(
        reader.advance(Chunk::from_start(0, file).unwrap()),
        Err(Error::MalformedFile(_))
    ));
}

#[test]
fn reader_rejects_incoherent_overlapping_chunks() {
    let mut reader = Reader::new();
    assert!(reader
        .advance(Chunk::from_start(0, vec![1, 2, 3, 4]).unwrap())
        .is_ok());
    let err = reader
        .advance(Chunk::from_start(2, vec![9, 4]).unwrap())
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
}

#[test]
fn reader_requests_referenced_arrays_incrementally() {
    let file = reader_file((40, 40), (16, 16), &[((0, 0), b"a")]);
    let mut reader = Reader::new();
    let status = reader
        .advance(Chunk::from_start(0, file[..16].to_vec()).unwrap())
        .unwrap();
    let ReadStatus::Need { req, .. } = status else {
        panic!("expected request")
    };
    assert!(req.start() >= 16);
}

#[test]
fn reader_accepts_full_file_after_prefix_chunk() {
    let file = reader_file((16, 16), (16, 16), &[((0, 0), b"tile")]);
    let mut reader = Reader::new();

    assert!(matches!(
        reader
            .advance(Chunk::from_start(0, file[..16].to_vec()).unwrap())
            .unwrap(),
        ReadStatus::Need { .. }
    ));
    assert!(matches!(
        reader.advance(Chunk::from_start(0, file).unwrap()).unwrap(),
        ReadStatus::Done { .. }
    ));
    assert_eq!(reader.zif().unwrap().dimensions(), (16, 16));
}

#[test]
fn reader_reads_metadata_from_truncated_reallife_fixture() {
    let file = include_bytes!("reallife.zif");
    let mut reader = Reader::new();
    let mut status = reader
        .advance(Chunk::from_start(0, file.to_vec()).unwrap())
        .unwrap();

    while let ReadStatus::Need { req, .. } = &status {
        let range = req.range();
        let Ok(end) = usize::try_from(range.end) else {
            break;
        };
        if end > file.len() {
            break;
        }
        let start = usize::try_from(range.start).unwrap();
        status = reader
            .advance(Chunk::new(range, file[start..end].to_vec()).unwrap())
            .unwrap();
    }

    let ReadStatus::Need { req, zif } = status else {
        panic!("truncated fixture should still need more data");
    };
    assert!(zif.is_some());
    assert!(usize::try_from(req.end()).unwrap() > file.len());
    let zif = reader.zif().unwrap();
    assert_eq!(zif.dimensions(), (7946, 10061));
    assert_eq!(zif.codec(), Codec::Jpeg);
    assert_eq!(zif.color_model(), ColorModel::YCbCr);
    assert_eq!(zif.channels(), 3);
    assert_eq!(zif.level_count(), 1);
    let level = zif.level(0).unwrap();
    assert_eq!(level.tile_size(), (256, 256));
    assert_eq!(level.tile_grid(), (32, 40));
    assert_eq!(level.tile_count(), 1280);

    let present_tiles: Vec<_> = zif
        .get_level_tiles(0)
        .unwrap()
        .filter(|tile| usize::try_from(tile.bytes().end).is_ok_and(|end| end <= file.len()))
        .collect();
    assert_eq!(
        present_tiles.len(),
        usize::try_from(level.tile_count()).unwrap()
    );
    let first = present_tiles.first().unwrap();
    assert_eq!(first.index(), 0);
    assert_eq!(first.position(), (0, 0));
    let range = first.bytes();
    let start = usize::try_from(range.start).unwrap();
    let end = usize::try_from(range.end).unwrap();
    assert!(start < end);
    assert!(file[start..end].iter().any(|&byte| byte != 0));
}

#[test]
fn reader_does_not_treat_start_zero_prefix_as_complete_file() {
    let file = reader_file((16, 16), (16, 16), &[((0, 0), b"tile")]);
    let mut reader = Reader::new();

    let status = reader
        .advance(Chunk::from_start(0, file[..16].to_vec()).unwrap())
        .unwrap();
    let ReadStatus::Need { req, .. } = status else {
        panic!("expected directory request");
    };
    let range = req.range();
    let start = usize::try_from(range.start).unwrap();
    let end = usize::try_from(range.end).unwrap();

    let mut status = reader
        .advance(Chunk::from_start(range.start, file[start..end].to_vec()).unwrap())
        .unwrap();
    while let ReadStatus::Need { req, .. } = status {
        let range = req.range();
        let start = usize::try_from(range.start).unwrap();
        let end = usize::try_from(range.end).unwrap();
        status = reader
            .advance(Chunk::from_start(range.start, file[start..end].to_vec()).unwrap())
            .unwrap();
    }
    assert!(matches!(status, ReadStatus::Done { .. }));
    assert_eq!(reader.zif().unwrap().dimensions(), (16, 16));
}

#[test]
fn tile_iteration_is_row_major_and_clips_edges() {
    let file = reader_file((40, 40), (16, 16), &[((0, 0), b"a")]);
    let zif = parse(&file);
    let coords: Vec<_> = zif
        .get_level_tiles(0)
        .unwrap()
        .map(|t| (t.col(), t.row(), t.index()))
        .collect();
    assert_eq!(coords[0], (0, 0, 0));
    assert_eq!(coords[1], (1, 0, 1));
    assert_eq!(coords[3], (0, 1, 3));
    let edge = zif.level(0).unwrap().tile(2, 2).unwrap();
    assert_eq!(edge.x(), 32);
    assert_eq!(edge.y(), 32);
    assert_eq!(edge.width(), 8);
    assert_eq!(edge.height(), 8);
    assert_eq!(edge.size(), (8, 8));
}

#[test]
fn cropped_tiles_clamp_out_of_bounds_and_reject_bad_region() {
    let file = reader_file((40, 40), (16, 16), &[((0, 0), b"a")]);
    let zif = parse(&file);
    assert_eq!(
        zif.get_cropped_level_tiles(0, (100..200, 0..10))
            .unwrap()
            .count(),
        0
    );
    let (start, end) = (20, 10);
    assert!(zif.get_cropped_level_tiles(0, (start..end, 0..10)).is_err());
}

#[test]
fn writer_validates_builder_inputs() {
    assert!(LevelSpec::new((0, 16), (16, 16)).is_err());
    assert!(LevelSpec::new((16, 16), (15, 16)).is_err());
    assert!(Writer::new().channels(2).is_err());
    assert!(Writer::new()
        .dimensions((16, 16))
        .tile_size((16, 16))
        .unwrap()
        .codec(Codec::Jpeg)
        .color_model(ColorModel::YCbCr)
        .build()
        .is_err());
    assert!(Writer::new()
        .dimensions((16, 16))
        .tile_size((16, 16))
        .unwrap()
        .codec(Codec::Jpeg)
        .color_model(ColorModel::BlackIsZero)
        .channels(3)
        .unwrap()
        .build()
        .is_err());
}

#[test]
fn writer_rejects_bad_tile_coordinates() {
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
    assert!(writer.put_tile((1, 0), b"x").is_err());
    assert!(writer.put_tile_at_level(1, (0, 0), b"x").is_err());
}

#[test]
fn writer_emits_bigtiff_compatible_tag_ids_types_and_order() {
    let file = reader_file(
        (40, 40),
        (16, 16),
        &[((0, 0), b"a"), ((1, 0), b"bb"), ((2, 2), b"ccc")],
    );
    let entries = first_directory_entries(&file);
    let codes: Vec<_> = entries.iter().map(|entry| entry.code).collect();

    assert_eq!(
        codes,
        vec![256, 257, 258, 259, 262, 277, 284, 322, 323, 324, 325, 530]
    );
    assert!(codes.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(entry(&entries, 258).ty, 3);
    assert_eq!(entry(&entries, 259).ty, 3);
    assert_eq!(entry(&entries, 262).ty, 3);
    assert_eq!(entry(&entries, 277).ty, 3);
    assert_eq!(entry(&entries, 284).ty, 3);
    assert_eq!(entry(&entries, 322).ty, 4);
    assert_eq!(entry(&entries, 323).ty, 4);
    assert_eq!(entry(&entries, 324).ty, 16);
    assert_eq!(entry(&entries, 324).count, 9);
    assert_eq!(entry(&entries, 325).ty, 4);
    assert_eq!(entry(&entries, 325).count, 9);
    assert_eq!(entry(&entries, 530).ty, 3);
    assert_eq!(entry(&entries, 530).count, 2);
}

#[test]
fn writer_uses_tiff_tile_offsets_before_tile_byte_counts() {
    let file = reader_file((32, 16), (16, 16), &[((0, 0), b"left"), ((1, 0), b"right")]);
    let entries = first_directory_entries(&file);
    let offsets = entry(&entries, 324);
    let counts = entry(&entries, 325);

    assert_eq!(offsets.ty, 16);
    assert_eq!(counts.ty, 4);
    let offsets_offset = usize::try_from(u64::from_le_bytes(offsets.slot)).unwrap();
    assert_eq!(u32::from_le_bytes(counts.slot[..4].try_into().unwrap()), 4);
    assert_eq!(u32::from_le_bytes(counts.slot[4..8].try_into().unwrap()), 5);
    let first_tile = usize::try_from(read_u64(&file, offsets_offset)).unwrap();
    let second_tile = usize::try_from(read_u64(&file, offsets_offset + 8)).unwrap();
    assert_eq!(&file[first_tile..first_tile + 4], b"left");
    assert_eq!(&file[second_tile..second_tile + 5], b"right");
}

#[test]
fn multi_level_writer_patches_next_directory_after_optional_tags() {
    let mut writer = Writer::new()
        .level(LevelSpec::new((32, 32), (16, 16)).unwrap())
        .level(LevelSpec::new((16, 16), (16, 16)).unwrap())
        .codec(Codec::Jpeg)
        .color_model(ColorModel::YCbCr)
        .channels(3)
        .unwrap()
        .build()
        .unwrap();
    let mut file = Vec::new();
    apply(
        &mut file,
        writer.put_tile_at_level(0, (0, 0), b"base").unwrap(),
    );
    apply(
        &mut file,
        writer.put_tile_at_level(1, (0, 0), b"top").unwrap(),
    );

    assert_raw_directory_chain(&file, 2, 12);
    let first_dir = usize::try_from(read_u64(&file, 8)).unwrap();
    let first_count = usize::try_from(read_u64(&file, first_dir)).unwrap();
    assert_eq!(first_count, 12);
    assert_eq!(entry(&first_directory_entries(&file), 530).count, 2);
    let second_dir = usize::try_from(read_u64(&file, first_dir + 8 + first_count * 20)).unwrap();
    assert_ne!(second_dir, 0);
    assert_eq!(read_u64(&file, second_dir), 12);
    let second_next = read_u64(&file, second_dir + 8 + 12 * 20);
    assert_eq!(second_next, 0);

    let zif = parse(&file);
    assert_eq!(zif.level_count(), 2);
}

#[test]
fn set_dimensions_preserves_existing_tile_positions() {
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
    apply(&mut file, writer.put_tile((0, 0), b"old").unwrap());
    apply(&mut file, writer.set_dimensions(0, (32, 16)).unwrap());
    let zif = parse(&file);
    assert_eq!(zif.dimensions(), (32, 16));
    assert_eq!(zif.level(0).unwrap().tile_grid(), (2, 1));
    let tile = zif.level(0).unwrap().tile(0, 0).unwrap();
    let bytes = tile.bytes();
    assert_eq!(
        &file[usize::try_from(bytes.start).unwrap()..usize::try_from(bytes.end).unwrap()],
        b"old"
    );
}

#[test]
fn set_dimensions_before_first_tile_does_not_overlap_later_tile_payload() {
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

    apply(&mut file, writer.set_dimensions(0, (32, 16)).unwrap());
    apply(&mut file, writer.put_tile((1, 0), b"new").unwrap());

    let zif = parse(&file);
    let tile = zif.level(0).unwrap().tile(1, 0).unwrap();
    let bytes = tile.bytes();
    assert_eq!(
        &file[usize::try_from(bytes.start).unwrap()..usize::try_from(bytes.end).unwrap()],
        b"new"
    );
}

#[test]
fn writer_replacing_tile_after_resize_points_to_new_payload() {
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

    apply(&mut file, writer.put_tile((0, 0), b"old").unwrap());
    apply(&mut file, writer.set_dimensions(0, (32, 16)).unwrap());
    apply(&mut file, writer.put_tile((0, 0), b"new payload").unwrap());

    let zif = parse(&file);
    let tile = zif.level(0).unwrap().tile(0, 0).unwrap();
    let bytes = tile.bytes();
    assert_eq!(
        &file[usize::try_from(bytes.start).unwrap()..usize::try_from(bytes.end).unwrap()],
        b"new payload"
    );
}

#[test]
fn multi_level_writer_links_directories_and_classifies_pyramid() {
    let mut writer = Writer::new()
        .level(LevelSpec::new((32, 32), (16, 16)).unwrap())
        .level(LevelSpec::new((16, 16), (16, 16)).unwrap())
        .codec(Codec::Jpeg)
        .color_model(ColorModel::YCbCr)
        .channels(3)
        .unwrap()
        .build()
        .unwrap();
    let mut file = Vec::new();
    apply(
        &mut file,
        writer.put_tile_at_level(0, (0, 0), b"base").unwrap(),
    );
    apply(
        &mut file,
        writer.put_tile_at_level(1, (0, 0), b"top").unwrap(),
    );
    let zif = parse(&file);
    assert_eq!(zif.level_count(), 2);
    assert_eq!(zif.chain_kind(), ChainKind::Pyramid);
    assert_eq!(zif.level(1).unwrap().dimensions(), (16, 16));
}

#[test]
fn reader_rejects_overflowing_tile_byte_range() {
    const ENTRY_LEN: usize = 20;
    const TYPE_U16: u16 = 3;
    const TYPE_U32: u16 = 4;
    const TYPE_U64: u16 = 16;

    fn push_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }
    fn push_u64(out: &mut Vec<u8>, value: u64) {
        out.extend_from_slice(&value.to_le_bytes());
    }
    fn entry(out: &mut Vec<u8>, code: u16, ty: u16, count: u64, slot: [u8; 8]) {
        push_u16(out, code);
        push_u16(out, ty);
        push_u64(out, count);
        out.extend_from_slice(&slot);
    }
    fn entry_u16(out: &mut Vec<u8>, code: u16, value: u16) {
        let mut slot = [0; 8];
        slot[..2].copy_from_slice(&value.to_le_bytes());
        entry(out, code, TYPE_U16, 1, slot);
    }
    fn entry_u32(out: &mut Vec<u8>, code: u16, value: u32) {
        let mut slot = [0; 8];
        slot[..4].copy_from_slice(&value.to_le_bytes());
        entry(out, code, TYPE_U32, 1, slot);
    }
    fn entry_u64(out: &mut Vec<u8>, code: u16, value: u64) {
        entry(out, code, TYPE_U64, 1, value.to_le_bytes());
    }

    let mut file = Vec::from([
        0x49, 0x49, 0x2b, 0x00, 0x08, 0x00, 0x00, 0x00, 16, 0, 0, 0, 0, 0, 0, 0,
    ]);
    push_u64(&mut file, 11);
    entry_u32(&mut file, 256, 16);
    entry_u32(&mut file, 257, 16);
    entry_u16(&mut file, 258, 8);
    entry_u16(&mut file, 259, 7);
    entry_u16(&mut file, 262, 6);
    entry_u16(&mut file, 277, 3);
    entry_u16(&mut file, 284, 1);
    entry_u32(&mut file, 322, 16);
    entry_u32(&mut file, 323, 16);
    entry_u64(
        &mut file,
        324,
        u64::try_from(16 + 8 + 11 * ENTRY_LEN + 8).unwrap(),
    );
    entry_u64(&mut file, 325, u64::MAX);
    push_u64(&mut file, 0);

    let mut reader = Reader::new();
    let err = reader
        .advance(Chunk::from_start(0, file).unwrap())
        .unwrap_err();
    assert!(matches!(
        err,
        Error::MalformedFile("tile byte count exceeds u32")
    ));
}
