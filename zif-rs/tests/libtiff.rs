#![allow(unsafe_code)]

use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use zif_tiff::{Codec, ColorModel, LevelConfig, WriteBatch, Writer};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
const JPEG_GRAY_16: &[u8] = include_bytes!("jpeg-gray-16.jpg");
const JPEG_YCBCR_420_16: &[u8] = include_bytes!("jpeg-ycbcr-420-16.jpg");

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
    apply(
        &mut file,
        writer.put_tile((0, 0), JPEG_YCBCR_420_16).unwrap(),
    );
    apply(
        &mut file,
        writer.put_tile((1, 0), JPEG_YCBCR_420_16).unwrap(),
    );
    apply(
        &mut file,
        writer.put_tile((2, 2), JPEG_YCBCR_420_16).unwrap(),
    );

    assert_libtiff_reads(&file, &[(40, 40, 16, 16, 3, ColorModel::YCbCr)]);
}

#[test]
fn ycbcr_420_jpeg_writer_output_is_readable_by_libtiff() {
    let mut writer = Writer::new()
        .dimensions((32, 16))
        .tile_size((16, 16))
        .unwrap()
        .codec(Codec::Jpeg)
        .color_model(ColorModel::YCbCr)
        .channels(3)
        .unwrap()
        .build()
        .unwrap();
    let mut file = Vec::new();
    apply(
        &mut file,
        writer.put_tile((0, 0), JPEG_YCBCR_420_16).unwrap(),
    );
    apply(
        &mut file,
        writer.put_tile((1, 0), JPEG_YCBCR_420_16).unwrap(),
    );

    assert_libtiff_reads(&file, &[(32, 16, 16, 16, 3, ColorModel::YCbCr)]);
}

#[test]
fn grayscale_jpeg_writer_output_is_readable_by_libtiff() {
    let mut writer = Writer::new()
        .dimensions((16, 16))
        .tile_size((16, 16))
        .unwrap()
        .codec(Codec::Jpeg)
        .color_model(ColorModel::BlackIsZero)
        .channels(1)
        .unwrap()
        .build()
        .unwrap();
    let mut file = Vec::new();
    apply(&mut file, writer.put_tile((0, 0), JPEG_GRAY_16).unwrap());

    assert_libtiff_reads(&file, &[(16, 16, 16, 16, 1, ColorModel::BlackIsZero)]);
}

#[test]
fn multi_level_writer_output_is_readable_by_libtiff() {
    let mut writer = Writer::new()
        .level(LevelConfig::new((32, 32), (16, 16)).unwrap())
        .level(LevelConfig::new((16, 16), (16, 16)).unwrap())
        .codec(Codec::Jpeg)
        .color_model(ColorModel::YCbCr)
        .channels(3)
        .unwrap()
        .build()
        .unwrap();
    let mut file = Vec::new();
    apply(
        &mut file,
        writer
            .put_tile_at_level(0, (0, 0), JPEG_YCBCR_420_16)
            .unwrap(),
    );
    apply(
        &mut file,
        writer
            .put_tile_at_level(1, (0, 0), JPEG_YCBCR_420_16)
            .unwrap(),
    );

    assert_libtiff_reads(
        &file,
        &[
            (32, 32, 16, 16, 3, ColorModel::YCbCr),
            (16, 16, 16, 16, 3, ColorModel::YCbCr),
        ],
    );
}

fn assert_libtiff_reads(file: &[u8], expected: &[(u32, u32, u32, u32, u16, ColorModel)]) {
    let path = temp_tiff_path();
    fs::write(&path, file).unwrap();
    let path = CString::new(path.to_string_lossy().as_bytes()).unwrap();
    let mode = CString::new("r").unwrap();

    unsafe {
        let tiff = libtiff_sys::TIFFOpen(path.as_ptr(), mode.as_ptr());
        assert!(!tiff.is_null(), "libtiff failed to open writer output");

        for (directory, &(width, height, tile_width, tile_height, samples, color_model)) in
            expected.iter().enumerate()
        {
            assert_u32_tag(tiff, libtiff_sys::TIFFTAG_IMAGEWIDTH, width);
            assert_u32_tag(tiff, libtiff_sys::TIFFTAG_IMAGELENGTH, height);
            assert_u16_tag(tiff, libtiff_sys::TIFFTAG_BITSPERSAMPLE, 8);
            assert_u16_tag(tiff, libtiff_sys::TIFFTAG_SAMPLESPERPIXEL, samples);
            assert_u16_tag(
                tiff,
                libtiff_sys::TIFFTAG_PHOTOMETRIC,
                photometric(color_model),
            );
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

fn photometric(color_model: ColorModel) -> u16 {
    match color_model {
        ColorModel::WhiteIsZero => u16::try_from(libtiff_sys::PHOTOMETRIC_MINISWHITE).unwrap(),
        ColorModel::BlackIsZero => u16::try_from(libtiff_sys::PHOTOMETRIC_MINISBLACK).unwrap(),
        ColorModel::Rgb => u16::try_from(libtiff_sys::PHOTOMETRIC_RGB).unwrap(),
        ColorModel::YCbCr => u16::try_from(libtiff_sys::PHOTOMETRIC_YCBCR).unwrap(),
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
