#![no_main]
#![allow(unsafe_code)]

use libfuzzer_sys::arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use std::ffi::CString;
use std::fs;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use zif::{
    ChainKind, Chunk, Codec, ColorModel, LevelSpec, ReadStatus, Reader, WriteBatch, WriteOp,
    Writer,
};

const ENTRY_LEN: usize = 20;
const TAG_WIDTH: u16 = 256;
const TAG_HEIGHT: u16 = 257;
const TAG_BITS: u16 = 258;
const TAG_CODEC: u16 = 259;
const TAG_COLOR: u16 = 262;
const TAG_CHANNELS: u16 = 277;
const TAG_INTERLEAVE: u16 = 284;
const TAG_TILE_WIDTH: u16 = 322;
const TAG_TILE_HEIGHT: u16 = 323;
const TAG_TILE_OFFSETS: u16 = 324;
const TAG_TILE_COUNTS: u16 = 325;
const TAG_YCBCR_SUBSAMPLING: u16 = 530;

const TYPE_U16: u16 = 3;
const TYPE_U32: u16 = 4;
const TYPE_U64: u16 = 16;

static NEXT_LIBTIFF_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Arbitrary, Debug)]
struct Input {
    shape: Shape,
    codec: CodecChoice,
    color: ColorChoice,
    operations: Vec<Operation>,
    raw_chunks: Vec<RawChunk>,
}

#[derive(Arbitrary, Debug)]
enum Shape {
    Single {
        width: u8,
        height: u8,
        tile_width: u8,
        tile_height: u8,
    },
    Pyramid {
        base_width: u8,
        base_height: u8,
        levels: u8,
    },
    TimeSeries {
        width: u8,
        height: u8,
        levels: u8,
    },
    Collection {
        first_width: u8,
        first_height: u8,
        second_width: u8,
        second_height: u8,
        levels: u8,
    },
}

#[derive(Arbitrary, Debug)]
enum CodecChoice {
    Jpeg,
    Png,
    JpegXr,
    Jpeg2000,
}

#[derive(Arbitrary, Debug)]
enum ColorChoice {
    WhiteIsZero,
    BlackIsZero,
    Rgb,
    YCbCr,
}

#[derive(Arbitrary, Debug)]
enum Operation {
    PutTile {
        level: u8,
        col: u8,
        row: u8,
        bytes: Vec<u8>,
    },
    SetDimensions {
        width: u8,
        height: u8,
    },
    FeedWholeFile,
    FeedPrefix {
        len: u16,
    },
    FeedRange {
        start: u16,
        len: u16,
    },
    FeedRequested,
    FeedDefault,
}

#[derive(Arbitrary, Debug)]
struct RawChunk {
    start: u16,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
struct ExpectedLevel {
    dimensions: (u64, u64),
    tile_size: (u64, u64),
    tiles_across: u64,
    tiles_down: u64,
    payloads: BTreeMap<(u64, u64), Vec<u8>>,
}

struct RawEntry {
    code: u16,
    ty: u16,
    count: u64,
}

fuzz_target!(|data: &[u8]| {
    deterministic_writer_regressions();

    let Ok(input) = Input::arbitrary(&mut Unstructured::new(data)) else {
        return;
    };

    fuzz_raw_reader(&input.raw_chunks);

    let Some((level_specs, mut expected)) = make_levels(input.shape) else {
        return;
    };
    let (color_model, channels) = color_and_channels(input.color);
    let mut writer = Writer::new()
        .codec(codec(input.codec))
        .color_model(color_model)
        .channels(channels)
        .expect("generated channel counts are valid");
    for spec in level_specs {
        writer = writer.level(spec);
    }
    let Ok(mut writer) = writer.build() else {
        return;
    };

    let mut file = Vec::new();
    let mut reader = Reader::new();
    let mut last_request = None;

    for operation in input.operations.into_iter().take(96) {
        match operation {
            Operation::PutTile {
                level,
                col,
                row,
                mut bytes,
            } => {
                if bytes.is_empty() {
                    bytes.push(0);
                }
                bytes.truncate(256);
                let level = usize::from(level) % expected.len();
                let exp = &mut expected[level];
                let col = u64::from(col) % exp.tiles_across;
                let row = u64::from(row) % exp.tiles_down;
                if let Ok(batch) = writer.put_tile_at_level(level, (col, row), &bytes) {
                    apply(&mut file, batch);
                    exp.payloads.insert((col, row), bytes);
                    assert_full_parse_invariants(&file, &expected, color_model, channels);
                }
            }
            Operation::SetDimensions { width, height } => {
                if expected.len() == 1 {
                    let tile_size = expected[0].tile_size;
                    let dimensions = (dimension(width), dimension(height));
                    if let Ok(batch) = writer.set_dimensions(dimensions) {
                        apply(&mut file, batch);
                        resize_expected(&mut expected[0], dimensions, tile_size);
                        assert_full_parse_invariants(&file, &expected, color_model, channels);
                    }
                }
            }
            Operation::FeedWholeFile => {
                last_request = advance_and_check(&mut reader, chunk_from_file(&file, 0, file.len()), &file);
            }
            Operation::FeedPrefix { len } => {
                let end = usize::from(len).min(file.len());
                last_request = advance_and_check(&mut reader, chunk_from_file(&file, 0, end), &file);
            }
            Operation::FeedRange { start, len } => {
                if !file.is_empty() {
                    let start = usize::from(start) % file.len();
                    let end = start.saturating_add(usize::from(len) % 1024).min(file.len());
                    last_request = advance_and_check(
                        &mut reader,
                        chunk_from_file(&file, start, end),
                        &file,
                    );
                }
            }
            Operation::FeedRequested => {
                if let Some(range) = last_request.clone() {
                    let start = usize::try_from(range.start).unwrap_or(usize::MAX).min(file.len());
                    let end = usize::try_from(range.end).unwrap_or(usize::MAX).min(file.len());
                    if start <= end {
                        last_request = advance_and_check(
                            &mut reader,
                            chunk_from_file(&file, start, end),
                            &file,
                        );
                    }
                }
            }
            Operation::FeedDefault => {
                last_request = advance_and_check(&mut reader, Some(Chunk::default()), &file);
            }
        }
    }

    if !file.is_empty() {
        assert_full_parse_invariants(&file, &expected, color_model, channels);
        assert_libtiff_reads_writer_output(&file, &expected, color_model, channels);
    }
});

fn deterministic_writer_regressions() {
    check_writer_case(
        vec![
            ((32, 32), (16, 16)),
            ((16, 16), (16, 16)),
        ],
        Codec::Jpeg,
        ColorModel::YCbCr,
        3,
        &[(0, 0, 0, b"base".as_slice()), (1, 0, 0, b"top".as_slice())],
    );
    check_writer_case(
        vec![((16, 16), (16, 16))],
        Codec::Jpeg,
        ColorModel::BlackIsZero,
        1,
        &[(0, 0, 0, b"gray".as_slice())],
    );
    check_writer_case(
        vec![((32, 16), (16, 16))],
        Codec::Png,
        ColorModel::Rgb,
        3,
        &[(0, 0, 0, b"left".as_slice()), (0, 1, 0, b"right".as_slice())],
    );
}

fn check_writer_case(
    specs: Vec<((u64, u64), (u32, u32))>,
    codec: Codec,
    color_model: ColorModel,
    channels: u16,
    tiles: &[(usize, u64, u64, &[u8])],
) {
    let mut expected: Vec<_> = specs
        .iter()
        .map(|&(dimensions, tile_size)| {
            ExpectedLevel {
                dimensions,
                tile_size: (u64::from(tile_size.0), u64::from(tile_size.1)),
                tiles_across: dimensions.0.div_ceil(u64::from(tile_size.0)),
                tiles_down: dimensions.1.div_ceil(u64::from(tile_size.1)),
                payloads: BTreeMap::new(),
            }
        })
        .collect();
    let mut builder = Writer::new().codec(codec).color_model(color_model).channels(channels).unwrap();
    for (dimensions, tile_size) in specs {
        builder = builder.level(LevelSpec::new(dimensions, tile_size).unwrap());
    }
    let mut writer = builder.build().expect("deterministic writer case is valid");
    let mut file = Vec::new();
    for &(level, col, row, bytes) in tiles {
        apply(
            &mut file,
            writer
                .put_tile_at_level(level, (col, row), bytes)
                .expect("deterministic tile coordinate is valid"),
        );
        expected[level].payloads.insert((col, row), bytes.to_vec());
        assert_full_parse_invariants(&file, &expected, color_model, channels);
    }
}

fn fuzz_raw_reader(chunks: &[RawChunk]) {
    let mut reader = Reader::new();
    for raw in chunks.iter().take(16) {
        let mut bytes = raw.bytes.clone();
        bytes.truncate(128);
        if let Ok(chunk) = Chunk::from_start(u64::from(raw.start), bytes) {
            let _ = reader.advance(chunk);
        }
    }
}

fn make_levels(shape: Shape) -> Option<(Vec<LevelSpec>, Vec<ExpectedLevel>)> {
    let mut dims = Vec::new();
    match shape {
        Shape::Single {
            width,
            height,
            tile_width,
            tile_height,
        } => dims.push((
            (dimension(width), dimension(height)),
            (tile_dimension(tile_width), tile_dimension(tile_height)),
        )),
        Shape::Pyramid {
            base_width,
            base_height,
            levels,
        } => {
            let mut width = dimension(base_width);
            let mut height = dimension(base_height);
            for _ in 0..(usize::from(levels % 4) + 1) {
                dims.push(((width, height), (16, 16)));
                width = width.div_ceil(2);
                height = height.div_ceil(2);
            }
        }
        Shape::TimeSeries {
            width,
            height,
            levels,
        } => {
            let dimensions = (dimension(width), dimension(height));
            for _ in 0..(usize::from(levels % 4) + 2) {
                dims.push((dimensions, (16, 16)));
            }
        }
        Shape::Collection {
            first_width,
            first_height,
            second_width,
            second_height,
            levels,
        } => {
            dims.push(((dimension(first_width), dimension(first_height)), (16, 16)));
            let second = (dimension(second_width), dimension(second_height));
            for i in 0..(usize::from(levels % 3) + 1) {
                let dimensions = if i % 2 == 0 {
                    second
                } else {
                    (second.0.saturating_add(16), second.1)
                };
                dims.push((dimensions, (16, 16)));
            }
        }
    }

    let mut specs = Vec::new();
    let mut expected = Vec::new();
    for (dimensions, tile_size) in dims {
        let spec = LevelSpec::new(
            dimensions,
            (
                u32::try_from(tile_size.0).ok()?,
                u32::try_from(tile_size.1).ok()?,
            ),
        )
        .ok()?;
        let mut level = ExpectedLevel {
            dimensions,
            tile_size,
            tiles_across: 0,
            tiles_down: 0,
            payloads: BTreeMap::new(),
        };
        resize_expected(&mut level, dimensions, tile_size);
        specs.push(spec);
        expected.push(level);
    }
    Some((specs, expected))
}

fn dimension(v: u8) -> u64 {
    1 + u64::from(v % 8) * 9
}

fn tile_dimension(v: u8) -> u64 {
    16 + u64::from(v % 4) * 16
}

fn codec(choice: CodecChoice) -> Codec {
    match choice {
        CodecChoice::Jpeg => Codec::Jpeg,
        CodecChoice::Png => Codec::Png,
        CodecChoice::JpegXr => Codec::JpegXr,
        CodecChoice::Jpeg2000 => Codec::Jpeg2000,
    }
}

fn color_and_channels(choice: ColorChoice) -> (ColorModel, u16) {
    match choice {
        ColorChoice::WhiteIsZero => (ColorModel::WhiteIsZero, 1),
        ColorChoice::BlackIsZero => (ColorModel::BlackIsZero, 1),
        ColorChoice::Rgb => (ColorModel::Rgb, 3),
        ColorChoice::YCbCr => (ColorModel::YCbCr, 3),
    }
}

fn resize_expected(level: &mut ExpectedLevel, dimensions: (u64, u64), tile_size: (u64, u64)) {
    level.dimensions = dimensions;
    level.tile_size = tile_size;
    level.tiles_across = dimensions.0.div_ceil(tile_size.0);
    level.tiles_down = dimensions.1.div_ceil(tile_size.1);
    level
        .payloads
        .retain(|&(col, row), _| col < level.tiles_across && row < level.tiles_down);
}

fn chunk_from_file(file: &[u8], start: usize, end: usize) -> Option<Chunk<Vec<u8>>> {
    Chunk::from_start(start as u64, file[start..end].to_vec()).ok()
}

fn advance_and_check(
    reader: &mut Reader,
    chunk: Option<Chunk<Vec<u8>>>,
    file: &[u8],
) -> Option<std::ops::Range<u64>> {
    let chunk = chunk?;
    match reader.advance(chunk) {
        Ok(ReadStatus::Need { req, .. }) => {
            let range = req.range();
            assert!(range.start <= range.end);
            Some(range)
        }
        Ok(ReadStatus::Done { .. }) => {
            let zif = reader.zif().expect("done reader has zif");
            assert_parsed_zif_invariants(file, zif);
            None
        }
        Err(_) => None,
    }
}

fn assert_full_parse_invariants(
    file: &[u8],
    expected: &[ExpectedLevel],
    color_model: ColorModel,
    channels: u16,
) {
    let mut reader = Reader::new();
    let status = reader
        .advance(Chunk::from_start(0, file.to_vec()).expect("full-file chunk is coherent"))
        .expect("writer output must parse");
    assert!(matches!(status, ReadStatus::Done { .. }));
    let zif = reader.zif().expect("done reader has zif");
    assert_parsed_zif_invariants(file, zif);
    assert_writer_directory_tags(file, expected);
    assert_eq!(zif.level_count(), expected.len());
    assert_eq!(zif.dimensions(), expected[0].dimensions);
    assert_eq!(zif.color_model(), color_model);
    assert_eq!(zif.channels(), channels);
    assert_eq!(zif.chain_kind(), expected_chain_kind(expected));

    for (level_index, exp) in expected.iter().enumerate() {
        let level = zif.level(level_index).expect("expected level exists");
        assert_eq!(level.dimensions(), exp.dimensions);
        assert_eq!(level.tile_size(), exp.tile_size);
        assert_eq!(level.tile_grid(), (exp.tiles_across, exp.tiles_down));
        assert_eq!(level.tile_count(), exp.tiles_across * exp.tiles_down);
        assert_eq!(level.color_model(), color_model);
        assert_eq!(level.channels(), channels);
        assert_eq!(zif.get_level_tiles(level_index).unwrap().count() as u64, level.tile_count());

        for row in 0..exp.tiles_down {
            for col in 0..exp.tiles_across {
                let tile = level.tile(col, row).expect("valid tile coordinate");
                assert_tile_geometry(&tile, exp);
                assert_eq!(tile.req().range(), tile.bytes());
                let bytes = tile.bytes();
                assert!(bytes.start <= bytes.end);
                let start = usize::try_from(bytes.start).expect("tile start fits usize");
                let end = usize::try_from(bytes.end).expect("tile end fits usize");
                assert!(end <= file.len());
                if let Some(payload) = exp.payloads.get(&(col, row)) {
                    assert_eq!(&file[start..end], payload.as_slice());
                } else {
                    assert_eq!(bytes.start, 0);
                    assert_eq!(bytes.end, 0);
                }
            }
        }

        assert_crop_count(zif, level_index, (0..exp.dimensions.0, 0..exp.dimensions.1));
        assert_crop_count(zif, level_index, (0..0, 0..exp.dimensions.1));
        assert_crop_count(
            zif,
            level_index,
            (
                exp.tile_size.0.saturating_sub(1)..exp.dimensions.0.saturating_add(exp.tile_size.0),
                exp.tile_size.1.saturating_sub(1)..exp.dimensions.1.saturating_add(exp.tile_size.1),
            ),
        );
    }
}

fn assert_writer_directory_tags(file: &[u8], expected: &[ExpectedLevel]) {
    let mut dir = read_u64(file, 8);
    for exp in expected {
        assert_ne!(dir, 0);
        let dir_offset = usize::try_from(dir).expect("directory offset fits usize");
        assert!(dir_offset + 8 <= file.len());
        let count = usize::try_from(read_u64(file, dir_offset)).expect("entry count fits usize");
        let has_ycbcr_subsampling = entry_color_model(file, dir_offset) == 6;
        assert_eq!(count, 11 + usize::from(has_ycbcr_subsampling));
        let entries: Vec<_> = (0..count)
            .map(|index| {
                let offset = dir_offset + 8 + index * ENTRY_LEN;
                assert!(offset + ENTRY_LEN <= file.len());
                RawEntry {
                    code: read_u16(file, offset),
                    ty: read_u16(file, offset + 2),
                    count: read_u64(file, offset + 4),
                }
            })
            .collect();
        let codes: Vec<_> = entries.iter().map(|entry| entry.code).collect();
        let mut expected_codes = vec![
            TAG_WIDTH,
            TAG_HEIGHT,
            TAG_BITS,
            TAG_CODEC,
            TAG_COLOR,
            TAG_CHANNELS,
            TAG_INTERLEAVE,
            TAG_TILE_WIDTH,
            TAG_TILE_HEIGHT,
            TAG_TILE_OFFSETS,
            TAG_TILE_COUNTS,
        ];
        if has_ycbcr_subsampling {
            expected_codes.push(TAG_YCBCR_SUBSAMPLING);
        }
        assert_eq!(codes, expected_codes);
        assert!(codes.windows(2).all(|pair| pair[0] < pair[1]));
        assert_entry(&entries, TAG_BITS, TYPE_U16, 1);
        assert_entry(&entries, TAG_CODEC, TYPE_U16, 1);
        assert_entry(&entries, TAG_COLOR, TYPE_U16, 1);
        assert_entry(&entries, TAG_CHANNELS, TYPE_U16, 1);
        assert_entry(&entries, TAG_INTERLEAVE, TYPE_U16, 1);
        assert_entry(&entries, TAG_TILE_WIDTH, TYPE_U32, 1);
        assert_entry(&entries, TAG_TILE_HEIGHT, TYPE_U32, 1);
        assert_entry(&entries, TAG_TILE_OFFSETS, TYPE_U64, exp.tiles_across * exp.tiles_down);
        assert_entry(&entries, TAG_TILE_COUNTS, TYPE_U32, exp.tiles_across * exp.tiles_down);
        if has_ycbcr_subsampling {
            assert_entry(&entries, TAG_YCBCR_SUBSAMPLING, TYPE_U16, 2);
        }
        dir = read_u64(file, dir_offset + 8 + count * ENTRY_LEN);
    }
    assert_eq!(dir, 0);
}

fn entry_color_model(file: &[u8], dir_offset: usize) -> u16 {
    let count = usize::try_from(read_u64(file, dir_offset)).expect("entry count fits usize");
    for index in 0..count {
        let offset = dir_offset + 8 + index * ENTRY_LEN;
        if read_u16(file, offset) == TAG_COLOR {
            return read_u16(file, offset + 12);
        }
    }
    0
}

fn assert_entry(entries: &[RawEntry], code: u16, ty: u16, count: u64) {
    let entry = entries.iter().find(|entry| entry.code == code).expect("tag is present");
    assert_eq!(entry.ty, ty);
    assert_eq!(entry.count, count);
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn assert_libtiff_reads_writer_output(
    file: &[u8],
    expected: &[ExpectedLevel],
    color_model: ColorModel,
    channels: u16,
) {
    if expected.len() > 4 || file.len() > 8192 {
        return;
    }
    let path = std::env::temp_dir().join(format!(
        "zif-fuzz-libtiff-{}.tif",
        NEXT_LIBTIFF_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    if fs::write(&path, file).is_err() {
        return;
    }
    let path = CString::new(path.to_string_lossy().as_bytes()).expect("temp path has no nul");
    let mode = CString::new("r").expect("mode has no nul");
    unsafe {
        let tiff = libtiff_sys::TIFFOpen(path.as_ptr(), mode.as_ptr());
        assert!(!tiff.is_null(), "libtiff failed to open writer output");
        for (index, exp) in expected.iter().enumerate() {
            assert_libtiff_u32(tiff, libtiff_sys::TIFFTAG_IMAGEWIDTH, exp.dimensions.0 as u32);
            assert_libtiff_u32(tiff, libtiff_sys::TIFFTAG_IMAGELENGTH, exp.dimensions.1 as u32);
            assert_libtiff_u16(tiff, libtiff_sys::TIFFTAG_BITSPERSAMPLE, 8);
            assert_libtiff_u16(tiff, libtiff_sys::TIFFTAG_SAMPLESPERPIXEL, channels);
            assert_libtiff_u16(tiff, libtiff_sys::TIFFTAG_PHOTOMETRIC, photometric(color_model));
            assert_libtiff_u32(tiff, libtiff_sys::TIFFTAG_TILEWIDTH, exp.tile_size.0 as u32);
            assert_libtiff_u32(tiff, libtiff_sys::TIFFTAG_TILELENGTH, exp.tile_size.1 as u32);
            assert_ne!(libtiff_sys::TIFFIsTiled(tiff), 0);
            assert_eq!(
                libtiff_sys::TIFFNumberOfTiles(tiff),
                (exp.tiles_across * exp.tiles_down) as u32
            );
            if index + 1 < expected.len() {
                assert_ne!(libtiff_sys::TIFFReadDirectory(tiff), 0);
            }
        }
        libtiff_sys::TIFFClose(tiff);
    }
}

fn photometric(color_model: ColorModel) -> u16 {
    match color_model {
        ColorModel::WhiteIsZero => libtiff_sys::PHOTOMETRIC_MINISWHITE as u16,
        ColorModel::BlackIsZero => libtiff_sys::PHOTOMETRIC_MINISBLACK as u16,
        ColorModel::Rgb => libtiff_sys::PHOTOMETRIC_RGB as u16,
        ColorModel::YCbCr => libtiff_sys::PHOTOMETRIC_YCBCR as u16,
    }
}

unsafe fn assert_libtiff_u16(tiff: *mut libtiff_sys::TIFF, tag: u32, expected: u16) {
    let mut value = 0u16;
    assert_ne!(libtiff_sys::TIFFGetField(tiff, tag, &mut value), 0);
    assert_eq!(value, expected);
}

unsafe fn assert_libtiff_u32(tiff: *mut libtiff_sys::TIFF, tag: u32, expected: u32) {
    let mut value = 0u32;
    assert_ne!(libtiff_sys::TIFFGetField(tiff, tag, &mut value), 0);
    assert_eq!(value, expected);
}

fn assert_parsed_zif_invariants(file: &[u8], zif: &zif::Zif) {
    assert!(zif.level_count() > 0);
    assert_eq!(zif.width(), zif.dimensions().0);
    assert_eq!(zif.height(), zif.dimensions().1);
    for level_index in 0..zif.level_count() {
        let level = zif.level(level_index).expect("level index in range");
        assert!(level.width() > 0);
        assert!(level.height() > 0);
        assert_eq!(level.dimensions(), (level.width(), level.height()));
        assert_eq!(level.tile_count(), level.tile_grid().0 * level.tile_grid().1);
        let mut seen = 0;
        for tile in zif.get_level_tiles(level_index).expect("level exists") {
            assert_eq!(tile.level(), level_index);
            assert_eq!(tile.index(), tile.row() * level.tile_grid().0 + tile.col());
            assert_eq!(tile.position(), (tile.x(), tile.y()));
            assert_eq!(tile.size(), (tile.width(), tile.height()));
            assert!(tile.width() > 0);
            assert!(tile.height() > 0);
            assert!(tile.x() < level.width());
            assert!(tile.y() < level.height());
            assert!(tile.x() + tile.width() <= level.width());
            assert!(tile.y() + tile.height() <= level.height());
            assert_eq!(tile.req().range(), tile.bytes());
            let bytes = tile.bytes();
            assert!(bytes.start <= bytes.end);
            assert!(usize::try_from(bytes.end).unwrap_or(usize::MAX) <= file.len());
            assert_eq!(tile.codec(), level.codec());
            seen += 1;
        }
        assert_eq!(seen, level.tile_count());
    }
}

fn assert_tile_geometry(tile: &zif::Tile<'_>, exp: &ExpectedLevel) {
    assert_eq!(tile.col(), tile.index() % exp.tiles_across);
    assert_eq!(tile.row(), tile.index() / exp.tiles_across);
    assert_eq!(tile.x(), tile.col() * exp.tile_size.0);
    assert_eq!(tile.y(), tile.row() * exp.tile_size.1);
    assert_eq!(tile.width(), exp.tile_size.0.min(exp.dimensions.0 - tile.x()));
    assert_eq!(tile.height(), exp.tile_size.1.min(exp.dimensions.1 - tile.y()));
}

fn assert_crop_count(zif: &zif::Zif, level_index: usize, region: (std::ops::Range<u64>, std::ops::Range<u64>)) {
    let level = zif.level(level_index).expect("level index in range");
    let expected = expected_crop_count(level, &region);
    let actual = zif
        .get_cropped_level_tiles(level_index, region)
        .expect("well-formed crop region")
        .count() as u64;
    assert_eq!(actual, expected);
}

fn expected_crop_count(level: &zif::Level, region: &(std::ops::Range<u64>, std::ops::Range<u64>)) -> u64 {
    let (tile_width, tile_height) = level.tile_size();
    let x0 = region.0.start.min(level.width());
    let x1 = region.0.end.min(level.width());
    let y0 = region.1.start.min(level.height());
    let y1 = region.1.end.min(level.height());
    if x0 >= x1 || y0 >= y1 {
        return 0;
    }
    let start_col = x0 / tile_width;
    let end_col = x1.div_ceil(tile_width).min(level.tile_grid().0);
    let start_row = y0 / tile_height;
    let end_row = y1.div_ceil(tile_height).min(level.tile_grid().1);
    (end_col - start_col) * (end_row - start_row)
}

fn expected_chain_kind(levels: &[ExpectedLevel]) -> ChainKind {
    if levels.len() <= 1 {
        return ChainKind::Pyramid;
    }
    if levels.iter().all(|l| l.dimensions == levels[0].dimensions) {
        return ChainKind::TimeSeries;
    }
    if levels.windows(2).all(|w| {
        w[1].dimensions.0 == w[0].dimensions.0.div_ceil(2)
            && w[1].dimensions.1 == w[0].dimensions.1.div_ceil(2)
    }) {
        ChainKind::Pyramid
    } else {
        ChainKind::Collection
    }
}

fn apply(file: &mut Vec<u8>, batch: WriteBatch) {
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
                let offset = usize::try_from(offset.get()).expect("fuzz offsets fit usize");
                assert!(file.len() >= offset + 8);
                file[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
            }
        }
    }
}
