#![allow(unsafe_code)]

use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use zif::{Codec, ColorModel, LevelSpec, WriteBatch, WriteOp, Writer};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

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
                let offset = usize::try_from(offset.get()).unwrap();
                file[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
            }
        }
    }
}

#[test]
fn writer_output_is_readable_by_libtiff() {
    let mut writer = Writer::new()
        .dimensions((40, 40))
        .tile_size((16, 16))
        .unwrap()
        .codec(Codec::Jpeg)
        .color_model(ColorModel::YCbCr)
        .channels(3)
        .unwrap()
        .build()
        .unwrap();
    let mut file = Vec::new();
    apply(&mut file, writer.put_tile((0, 0), b"a").unwrap());
    apply(&mut file, writer.put_tile((1, 0), b"bb").unwrap());
    apply(&mut file, writer.put_tile((2, 2), b"ccc").unwrap());

    assert_libtiff_reads(&file, &[(40, 40, 16, 16, 3)]);
}

#[test]
fn multi_level_writer_output_is_readable_by_libtiff() {
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

    assert_libtiff_reads(&file, &[(32, 32, 16, 16, 3), (16, 16, 16, 16, 3)]);
}

fn assert_libtiff_reads(file: &[u8], expected: &[(u32, u32, u32, u32, u16)]) {
    let path = temp_tiff_path();
    fs::write(&path, file).unwrap();
    let path = CString::new(path.to_string_lossy().as_bytes()).unwrap();
    let mode = CString::new("r").unwrap();

    unsafe {
        let tiff = libtiff_sys::TIFFOpen(path.as_ptr(), mode.as_ptr());
        assert!(!tiff.is_null(), "libtiff failed to open writer output");

        for (directory, &(width, height, tile_width, tile_height, samples)) in
            expected.iter().enumerate()
        {
            assert_u32_tag(tiff, libtiff_sys::TIFFTAG_IMAGEWIDTH, width);
            assert_u32_tag(tiff, libtiff_sys::TIFFTAG_IMAGELENGTH, height);
            assert_u16_tag(tiff, libtiff_sys::TIFFTAG_BITSPERSAMPLE, 8);
            assert_u16_tag(tiff, libtiff_sys::TIFFTAG_SAMPLESPERPIXEL, samples);
            assert_u16_tag(
                tiff,
                libtiff_sys::TIFFTAG_PLANARCONFIG,
                u16::try_from(libtiff_sys::PLANARCONFIG_CONTIG).unwrap(),
            );
            assert_u32_tag(tiff, libtiff_sys::TIFFTAG_TILEWIDTH, tile_width);
            assert_u32_tag(tiff, libtiff_sys::TIFFTAG_TILELENGTH, tile_height);
            assert_ne!(libtiff_sys::TIFFIsTiled(tiff), 0);
            assert_eq!(
                libtiff_sys::TIFFNumberOfTiles(tiff),
                width.div_ceil(tile_width) * height.div_ceil(tile_height)
            );

            if directory + 1 < expected.len() {
                assert_ne!(libtiff_sys::TIFFReadDirectory(tiff), 0);
            } else {
                assert_eq!(libtiff_sys::TIFFReadDirectory(tiff), 0);
            }
        }

        libtiff_sys::TIFFClose(tiff);
    }
}

unsafe fn assert_u16_tag(tiff: *mut libtiff_sys::TIFF, tag: u32, expected: u16) {
    let mut value = 0u16;
    assert_ne!(libtiff_sys::TIFFGetField(tiff, tag, &mut value), 0);
    assert_eq!(value, expected);
}

unsafe fn assert_u32_tag(tiff: *mut libtiff_sys::TIFF, tag: u32, expected: u32) {
    let mut value = 0u32;
    assert_ne!(libtiff_sys::TIFFGetField(tiff, tag, &mut value), 0);
    assert_eq!(value, expected);
}

fn temp_tiff_path() -> PathBuf {
    let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("zif-libtiff-{id}.tif"))
}
