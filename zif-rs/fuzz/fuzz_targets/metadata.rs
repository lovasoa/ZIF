#![no_main]

use libfuzzer_sys::arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use zif_tiff::{DataChunk, Error, ParseState, Parser};

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

const TYPE_U16: u16 = 3;
const TYPE_U32: u16 = 4;
const TYPE_U64: u16 = 16;

const VALID_HEADER: [u8; 16] = [
    0x49, 0x49, 0x2b, 0x00, 0x08, 0x00, 0x00, 0x00, 16, 0, 0, 0, 0, 0, 0, 0,
];

#[derive(Arbitrary, Debug)]
struct Input {
    mode: Mode,
    reads: Vec<ReadPlan>,
}

#[derive(Arbitrary, Debug)]
enum Mode {
    Raw { bytes: Vec<u8> },
    Structured { dirs: Vec<Dir>, trailing: Vec<u8> },
}

#[derive(Arbitrary, Debug)]
struct Dir {
    width: Scalar,
    height: Scalar,
    bits: EntryValue,
    codec: EntryValue,
    color: EntryValue,
    channels: EntryValue,
    interleave: EntryValue,
    tile_width: Scalar,
    tile_height: Scalar,
    offsets: ArrayValue,
    counts: ArrayValue,
    extras: Vec<ExtraEntry>,
    order: Order,
    next: NextDir,
}

#[derive(Arbitrary, Debug)]
struct Scalar {
    ty: ScalarTy,
    count: SmallCount,
    value: u32,
}

#[derive(Arbitrary, Debug)]
enum ScalarTy {
    U16,
    U32,
    U64,
    Unknown,
}

#[derive(Arbitrary, Debug)]
struct EntryValue {
    ty: ScalarTy,
    count: SmallCount,
    value: u16,
}

#[derive(Arbitrary, Debug)]
struct ArrayValue {
    ty: ArrayTy,
    count: CountChoice,
    storage: Storage,
    values: Vec<u64>,
}

#[derive(Arbitrary, Debug)]
enum ArrayTy {
    U32,
    U64,
    U16,
    Unknown,
}

#[derive(Arbitrary, Debug)]
enum CountChoice {
    Exact,
    Zero,
    One,
    Two,
    Small(u8),
    Huge(u16),
    Max,
}

#[derive(Arbitrary, Debug)]
enum Storage {
    Inline,
    Referenced,
    BadOffset(u64),
}

#[derive(Arbitrary, Debug)]
struct ExtraEntry {
    code: u16,
    ty: u16,
    count: u16,
    slot: u64,
}

#[derive(Arbitrary, Debug)]
enum Order {
    Sorted,
    Reversed,
    DuplicateRequired,
    ShuffleByInput,
}

#[derive(Arbitrary, Debug)]
enum NextDir {
    Normal,
    Zero,
    SelfCycle,
    FirstCycle,
    Offset(u64),
}

#[derive(Arbitrary, Debug)]
enum SmallCount {
    One,
    Zero,
    Two,
    Huge,
}

#[derive(Arbitrary, Debug)]
enum ReadPlan {
    Full,
    Prefix(u16),
    Range { start: u16, len: u16 },
    Requested,
    Empty,
    OverlapConflict { start: u16, len: u8 },
}

#[derive(Clone)]
struct Entry {
    code: u16,
    ty: u16,
    count: u64,
    slot: [u8; 8],
}

fuzz_target!(|data: &[u8]| {
    parse_full_and_check(&out_of_file_tile_range_file());

    let Ok(input) = Input::arbitrary(&mut Unstructured::new(data)) else {
        return;
    };

    let file = match input.mode {
        Mode::Raw { mut bytes } => {
            bytes.truncate(4096);
            bytes
        }
        Mode::Structured { dirs, mut trailing } => {
            trailing.truncate(512);
            build_structured(dirs, trailing)
        }
    };

    fuzz_incremental_reads(&file, input.reads);
    parse_full_and_check(&file);
});

fn out_of_file_tile_range_file() -> Vec<u8> {
    let mut file = Vec::from(VALID_HEADER);
    push_u64(&mut file, 11);
    push_entry(&mut file, &inline_u32(TAG_WIDTH, 16));
    push_entry(&mut file, &inline_u32(TAG_HEIGHT, 16));
    push_entry(&mut file, &inline_u16(TAG_BITS, 8));
    push_entry(&mut file, &inline_u16(TAG_CODEC, 7));
    push_entry(&mut file, &inline_u16(TAG_COLOR, 6));
    push_entry(&mut file, &inline_u16(TAG_CHANNELS, 3));
    push_entry(&mut file, &inline_u16(TAG_INTERLEAVE, 1));
    push_entry(&mut file, &inline_u32(TAG_TILE_WIDTH, 16));
    push_entry(&mut file, &inline_u32(TAG_TILE_HEIGHT, 16));
    push_entry(&mut file, &inline_u64(TAG_TILE_OFFSETS, u64::MAX - 7));
    push_entry(&mut file, &inline_u32_array(TAG_TILE_COUNTS, 8, 0));
    push_u64(&mut file, 0);
    file
}

fn build_structured(dirs: Vec<Dir>, trailing: Vec<u8>) -> Vec<u8> {
    let dirs: Vec<_> = dirs.into_iter().take(5).collect();
    let mut file = Vec::from([
        0x49, 0x49, 0x2b, 0x00, 0x08, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    if dirs.is_empty() {
        return file;
    }

    let mut plans = Vec::new();
    for dir in dirs {
        let entries = entries_for_dir(&dir);
        let dir_len = 8 + entries.len() * ENTRY_LEN + 8;
        plans.push((dir, entries, dir_len));
    }

    let mut next_dir_offset = 16u64;
    for (dir, entries, dir_len) in &mut plans {
        let this_dir = next_dir_offset;
        next_dir_offset = next_dir_offset.saturating_add(*dir_len as u64);
        for entry in entries.iter_mut() {
            if is_referenced_array(entry) {
                entry.slot = next_dir_offset.to_le_bytes();
                let bytes = array_bytes(entry, dir);
                next_dir_offset = next_dir_offset.saturating_add(bytes.len() as u64);
            }
        }
        if matches!(dir.offsets.storage, Storage::BadOffset(_)) {
            patch_bad_array_offset(entries, TAG_TILE_OFFSETS, &dir.offsets.storage);
        }
        if matches!(dir.counts.storage, Storage::BadOffset(_)) {
            patch_bad_array_offset(entries, TAG_TILE_COUNTS, &dir.counts.storage);
        }
        let _ = this_dir;
    }

    file[8..16].copy_from_slice(&16u64.to_le_bytes());
    let dir_offsets = compute_dir_offsets(&plans);
    for (index, (dir, entries, _)) in plans.iter().enumerate() {
        push_u64(&mut file, entries.len() as u64);
        for entry in entries {
            push_entry(&mut file, entry);
        }
        let next = match dir.next {
            NextDir::Normal => dir_offsets.get(index + 1).copied().unwrap_or(0),
            NextDir::Zero => 0,
            NextDir::SelfCycle => dir_offsets[index],
            NextDir::FirstCycle => 16,
            NextDir::Offset(v) => v,
        };
        push_u64(&mut file, next);
        for entry in entries {
            if is_referenced_array(entry) {
                file.extend_from_slice(&array_bytes(entry, dir));
            }
        }
    }
    file.extend_from_slice(&trailing);
    file.truncate(8192);
    file
}

fn entries_for_dir(dir: &Dir) -> Vec<Entry> {
    let tile_count = expected_tile_count(dir);
    let mut entries = vec![
        scalar_entry(TAG_WIDTH, &dir.width),
        scalar_entry(TAG_HEIGHT, &dir.height),
        value_entry(TAG_BITS, &dir.bits),
        value_entry(TAG_CODEC, &dir.codec),
        value_entry(TAG_COLOR, &dir.color),
        value_entry(TAG_CHANNELS, &dir.channels),
        value_entry(TAG_INTERLEAVE, &dir.interleave),
        scalar_entry(TAG_TILE_WIDTH, &dir.tile_width),
        scalar_entry(TAG_TILE_HEIGHT, &dir.tile_height),
        array_entry(TAG_TILE_OFFSETS, &dir.offsets, tile_count),
        array_entry(TAG_TILE_COUNTS, &dir.counts, tile_count),
    ];
    for extra in dir.extras.iter().take(8) {
        entries.push(Entry {
            code: extra.code,
            ty: extra.ty,
            count: u64::from(extra.count),
            slot: extra.slot.to_le_bytes(),
        });
    }
    match dir.order {
        Order::Sorted => entries.sort_by_key(|e| e.code),
        Order::Reversed => entries.sort_by_key(|e| std::cmp::Reverse(e.code)),
        Order::DuplicateRequired => {
            entries.push(entries[0].clone());
            entries.sort_by_key(|e| e.code);
        }
        Order::ShuffleByInput => {
            let len = entries.len();
            entries.rotate_left(usize::from(dir.bits.value % len as u16));
        }
    }
    entries
}

fn scalar_entry(code: u16, scalar: &Scalar) -> Entry {
    let ty = scalar_ty(&scalar.ty);
    let count = small_count(&scalar.count);
    let mut slot = [0; 8];
    match ty {
        TYPE_U16 => slot[..2].copy_from_slice(&(scalar.value as u16).to_le_bytes()),
        TYPE_U32 => slot[..4].copy_from_slice(&scalar.value.to_le_bytes()),
        _ => slot.copy_from_slice(&u64::from(scalar.value).to_le_bytes()),
    }
    Entry {
        code,
        ty,
        count,
        slot,
    }
}

fn value_entry(code: u16, value: &EntryValue) -> Entry {
    let ty = scalar_ty(&value.ty);
    let count = small_count(&value.count);
    let mut slot = [0; 8];
    slot[..2].copy_from_slice(&value.value.to_le_bytes());
    Entry {
        code,
        ty,
        count,
        slot,
    }
}

fn array_entry(code: u16, array: &ArrayValue, exact_count: u64) -> Entry {
    let ty = match array.ty {
        ArrayTy::U32 => TYPE_U32,
        ArrayTy::U64 => TYPE_U64,
        ArrayTy::U16 => TYPE_U16,
        ArrayTy::Unknown => 99,
    };
    let count = match array.count {
        CountChoice::Exact => exact_count,
        CountChoice::Zero => 0,
        CountChoice::One => 1,
        CountChoice::Two => 2,
        CountChoice::Small(v) => u64::from(v % 16),
        CountChoice::Huge(v) => 4090 + u64::from(v % 16),
        CountChoice::Max => u64::MAX,
    };
    let mut slot = [0; 8];
    let first = array.values.first().copied().unwrap_or(0);
    match (ty, &array.storage) {
        (TYPE_U32, Storage::Inline) => {
            slot[..4].copy_from_slice(&(first as u32).to_le_bytes());
            let second = array.values.get(1).copied().unwrap_or(0) as u32;
            slot[4..8].copy_from_slice(&second.to_le_bytes());
        }
        (_, Storage::Inline) => slot.copy_from_slice(&first.to_le_bytes()),
        (_, Storage::BadOffset(offset)) => slot.copy_from_slice(&offset.to_le_bytes()),
        (_, Storage::Referenced) => {}
    }
    Entry {
        code,
        ty,
        count,
        slot,
    }
}

fn expected_tile_count(dir: &Dir) -> u64 {
    let width = valid_scalar_value(&dir.width).max(1);
    let height = valid_scalar_value(&dir.height).max(1);
    let tile_width = valid_scalar_value(&dir.tile_width).max(1);
    let tile_height = valid_scalar_value(&dir.tile_height).max(1);
    width
        .div_ceil(tile_width)
        .saturating_mul(height.div_ceil(tile_height))
}

fn valid_scalar_value(scalar: &Scalar) -> u64 {
    match scalar.ty {
        ScalarTy::U16 => u64::from(scalar.value as u16),
        ScalarTy::U32 => u64::from(scalar.value),
        _ => u64::from(scalar.value),
    }
}

fn scalar_ty(ty: &ScalarTy) -> u16 {
    match ty {
        ScalarTy::U16 => TYPE_U16,
        ScalarTy::U32 => TYPE_U32,
        ScalarTy::U64 => TYPE_U64,
        ScalarTy::Unknown => 99,
    }
}

fn small_count(count: &SmallCount) -> u64 {
    match count {
        SmallCount::One => 1,
        SmallCount::Zero => 0,
        SmallCount::Two => 2,
        SmallCount::Huge => 4097,
    }
}

fn compute_dir_offsets(plans: &[(Dir, Vec<Entry>, usize)]) -> Vec<u64> {
    let mut offsets = Vec::new();
    let mut cursor = 16u64;
    for (_, entries, dir_len) in plans {
        offsets.push(cursor);
        cursor = cursor.saturating_add(*dir_len as u64);
        for entry in entries {
            if is_referenced_array(entry) {
                cursor = cursor.saturating_add(array_byte_len(entry));
            }
        }
    }
    offsets
}

fn is_referenced_array(entry: &Entry) -> bool {
    match entry.code {
        TAG_TILE_OFFSETS => entry.ty == TYPE_U64 && entry.count > 1,
        TAG_TILE_COUNTS => entry.ty == TYPE_U32 && entry.count > 2,
        _ => false,
    }
}

fn array_byte_len(entry: &Entry) -> u64 {
    let element = if entry.ty == TYPE_U64 { 8 } else { 4 };
    entry.count.min(512).saturating_mul(element)
}

fn array_bytes(entry: &Entry, dir: &Dir) -> Vec<u8> {
    let source = if entry.code == TAG_TILE_OFFSETS {
        &dir.offsets.values
    } else {
        &dir.counts.values
    };
    let mut out = Vec::new();
    for i in 0..entry.count.min(512) {
        let value = source.get(i as usize).copied().unwrap_or(i);
        if entry.ty == TYPE_U64 {
            push_u64(&mut out, value);
        } else {
            push_u32(&mut out, value as u32);
        }
    }
    out
}

fn patch_bad_array_offset(entries: &mut [Entry], code: u16, storage: &Storage) {
    let Storage::BadOffset(offset) = storage else {
        return;
    };
    if let Some(entry) = entries.iter_mut().find(|e| e.code == code) {
        entry.slot.copy_from_slice(&offset.to_le_bytes());
    }
}

fn push_entry(out: &mut Vec<u8>, entry: &Entry) {
    push_u16(out, entry.code);
    push_u16(out, entry.ty);
    push_u64(out, entry.count);
    out.extend_from_slice(&entry.slot);
}

fn inline_u16(code: u16, value: u16) -> Entry {
    let mut slot = [0; 8];
    slot[..2].copy_from_slice(&value.to_le_bytes());
    Entry {
        code,
        ty: TYPE_U16,
        count: 1,
        slot,
    }
}

fn inline_u32(code: u16, value: u32) -> Entry {
    let mut slot = [0; 8];
    slot[..4].copy_from_slice(&value.to_le_bytes());
    Entry {
        code,
        ty: TYPE_U32,
        count: 1,
        slot,
    }
}

fn inline_u64(code: u16, value: u64) -> Entry {
    Entry {
        code,
        ty: TYPE_U64,
        count: 1,
        slot: value.to_le_bytes(),
    }
}

fn inline_u32_array(code: u16, first: u32, second: u32) -> Entry {
    let mut slot = [0; 8];
    slot[..4].copy_from_slice(&first.to_le_bytes());
    slot[4..8].copy_from_slice(&second.to_le_bytes());
    Entry {
        code,
        ty: TYPE_U32,
        count: 1,
        slot,
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn fuzz_incremental_reads(file: &[u8], reads: Vec<ReadPlan>) {
    let mut parser = Parser::new();
    let mut request = None;
    for read in reads.into_iter().take(64) {
        let chunk = match read {
            ReadPlan::Full => chunk(file, 0, file.len()),
            ReadPlan::Prefix(len) => chunk(file, 0, usize::from(len).min(file.len())),
            ReadPlan::Range { start, len } => {
                if file.is_empty() {
                    None
                } else {
                    let start = usize::from(start) % file.len();
                    let end = start.saturating_add(usize::from(len)).min(file.len());
                    chunk(file, start, end)
                }
            }
            ReadPlan::Requested => request.clone().and_then(|range: std::ops::Range<u64>| {
                let start = usize::try_from(range.start)
                    .unwrap_or(usize::MAX)
                    .min(file.len());
                let end = usize::try_from(range.end)
                    .unwrap_or(usize::MAX)
                    .min(file.len());
                if start <= end {
                    chunk(file, start, end)
                } else {
                    None
                }
            }),
            ReadPlan::Empty => Some(DataChunk::default()),
            ReadPlan::OverlapConflict { start, len } => {
                if file.is_empty() {
                    None
                } else {
                    let start = usize::from(start) % file.len();
                    let end = start
                        .saturating_add(usize::from(len).max(1))
                        .min(file.len());
                    let mut bytes = file[start..end].to_vec();
                    if let Some(first) = bytes.first_mut() {
                        *first ^= 0xff;
                    }
                    DataChunk::from_start(start as u64, bytes).ok()
                }
            }
        };
        if let Some(chunk) = chunk {
            match parser.feed(chunk) {
                Ok(ParseState::Need { range, .. }) => {
                    let range = range.range();
                    assert!(range.start <= range.end);
                    request = Some(range);
                }
                Ok(ParseState::Done { .. }) => {
                    check_done_parser(&parser, file);
                    request = None;
                }
                Err(Error::Incomplete) => {
                    panic!("advance should return NeedMore instead of Incomplete")
                }
                Err(_) => {}
            }
        }
    }
}

fn chunk(file: &[u8], start: usize, end: usize) -> Option<DataChunk<Vec<u8>>> {
    DataChunk::from_start(start as u64, file[start..end].to_vec()).ok()
}

fn parse_full_and_check(file: &[u8]) {
    let mut parser = Parser::new();
    match parser.feed(DataChunk::from_start(0, file.to_vec()).expect("full-file chunk is coherent"))
    {
        Ok(ParseState::Done { .. }) => check_done_parser(&parser, file),
        Ok(ParseState::Need { range, .. }) => assert!(range.start() <= range.end()),
        Err(Error::Incomplete) => panic!("advance should return NeedMore instead of Incomplete"),
        Err(_) => {}
    }
}

fn check_done_parser(parser: &Parser, file: &[u8]) {
    let image = parser.image().expect("done parser has image");
    assert!(image.level_count() > 0);
    assert_eq!(image.dimensions(), (image.width(), image.height()));
    for level_index in 0..image.level_count() {
        let level = image.level(level_index).expect("level index exists");
        assert!(level.width() > 0);
        assert!(level.height() > 0);
        assert!(level.tile_size().0 > 0);
        assert!(level.tile_size().1 > 0);
        assert_eq!(
            level.tile_count(),
            level.tile_grid().0 * level.tile_grid().1
        );
        let tiles: Vec<_> = image
            .level_tiles(level_index)
            .expect("level exists")
            .collect();
        assert_eq!(tiles.len() as u64, level.tile_count());
        for tile in tiles {
            assert_eq!(tile.level(), level_index);
            assert_eq!(tile.index(), tile.row() * level.tile_grid().0 + tile.col());
            assert!(tile.x() < level.width());
            assert!(tile.y() < level.height());
            assert!(tile.width() > 0);
            assert!(tile.height() > 0);
            assert!(tile.x() + tile.width() <= level.width());
            assert!(tile.y() + tile.height() <= level.height());
            assert_eq!(tile.position(), (tile.x(), tile.y()));
            assert_eq!(tile.size(), (tile.width(), tile.height()));
            assert_eq!(tile.range().range(), tile.byte_range());
            assert!(usize::try_from(tile.byte_range().end).unwrap_or(usize::MAX) <= file.len());
        }
    }
}
