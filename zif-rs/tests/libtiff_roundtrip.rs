#![allow(unsafe_code)]

use std::ffi::CString;
use std::fs;
use std::os::raw::{c_int, c_ulong};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use zif_tiff::{
    Codec, ColorModel, DataChunk, ImageKind, LevelConfig, ParseState, Parser, WriteBatch, Writer,
};

use mozjpeg_sys::{
    jpeg_c_set_int_param, jpeg_compress_struct, jpeg_create_compress, jpeg_destroy_compress,
    jpeg_error_mgr, jpeg_finish_compress, jpeg_mem_dest, jpeg_set_colorspace, jpeg_set_defaults,
    jpeg_set_quality, jpeg_start_compress, jpeg_std_error, jpeg_write_scanlines, JCP_FASTEST,
    JCS_GRAYSCALE, JCS_RGB, JINT_COMPRESS_PROFILE,
};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

fn temp_path(id: u64) -> PathBuf {
    std::env::temp_dir().join(format!("zif-libtiff-roundtrip-{id}.tif"))
}

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

// ── mozjpeg helpers ────────────────────────────────────────────────────

unsafe fn compress_rgb(width: u32, height: u32, r: u8, g: u8, b: u8) -> Vec<u8> {
    let pixel_count = (width * height) as usize;
    let mut pixels = Vec::with_capacity(pixel_count * 3);
    for _ in 0..pixel_count {
        pixels.extend_from_slice(&[r, g, b]);
    }
    compress_jpeg(&pixels, width, height, 3)
}

#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
unsafe fn compress_jpeg(data: &[u8], width: u32, height: u32, channels: i32) -> Vec<u8> {
    let mut err: jpeg_error_mgr = std::mem::zeroed();
    jpeg_std_error(&mut err);
    let mut cinfo: jpeg_compress_struct = std::mem::zeroed();
    cinfo.common.err = &mut err;
    jpeg_create_compress(&mut cinfo);

    let mut out_buf: *mut u8 = std::ptr::null_mut();
    let mut out_size: c_ulong = 0;
    jpeg_mem_dest(&mut cinfo, &mut out_buf, &mut out_size);

    cinfo.image_width = width;
    cinfo.image_height = height;
    cinfo.input_components = channels as c_int;
    cinfo.in_color_space = if channels == 1 {
        JCS_GRAYSCALE
    } else {
        JCS_RGB
    };

    jpeg_set_defaults(&mut cinfo);
    // Force baseline (non‑progressive) and 1:1 chroma sampling — both are
    // required for JPEG‑in‑TIFF compatibility.
    jpeg_c_set_int_param(&mut cinfo, JINT_COMPRESS_PROFILE, JCP_FASTEST as c_int);
    cinfo.num_scans = 0;
    cinfo.scan_info = std::ptr::null();
    cinfo.optimize_coding = 1; // embed optimal Huffman tables in each scan
                               // Encode in RGB so that PHOTOMETRIC=RGB matches the bitstream.
    jpeg_set_colorspace(&mut cinfo, JCS_RGB);
    jpeg_set_quality(&mut cinfo, 75, 1);
    jpeg_start_compress(&mut cinfo, 1);

    let row_stride = width as usize * channels as usize;
    while cinfo.next_scanline < cinfo.image_height {
        let offset = cinfo.next_scanline as usize * row_stride;
        let row_ptr = data[offset..offset + row_stride].as_ptr();
        let samparray: *const *const u8 = &row_ptr;
        jpeg_write_scanlines(&mut cinfo, samparray, 1);
    }

    jpeg_finish_compress(&mut cinfo);
    let result = std::slice::from_raw_parts(out_buf, out_size as usize).to_vec();
    jpeg_destroy_compress(&mut cinfo);
    result
}

// ── libtiff helpers ────────────────────────────────────────────────────

unsafe fn set_u32(tif: *mut libtiff_sys::TIFF, tag: u32, value: u32) {
    assert_ne!(
        libtiff_sys::TIFFSetField(tif, tag, value),
        0,
        "TIFFSetField({tag}) failed"
    );
}

unsafe fn set_u16(tif: *mut libtiff_sys::TIFF, tag: u32, value: u16) {
    assert_ne!(
        libtiff_sys::TIFFSetField(tif, tag, u32::from(value)),
        0,
        "TIFFSetField({tag}) failed"
    );
}

/// Write pre‑compressed JPEG data via `TIFFWriteRawTile` so libtiff does
/// not touch the codec path at all.
unsafe fn write_raw_tile(tif: *mut libtiff_sys::TIFF, tile: u32, data: &[u8]) {
    let written = libtiff_sys::TIFFWriteRawTile(
        tif,
        tile,
        data.as_ptr().cast_mut().cast::<std::ffi::c_void>(),
        libtiff_sys::tmsize_t::try_from(data.len()).unwrap(),
    );
    assert!(written > 0, "TIFFWriteRawTile({tile}) failed");
}

unsafe fn assert_u16_tag(tif: *mut libtiff_sys::TIFF, tag: u32, expected: u16) {
    let mut value = 0u16;
    assert_ne!(
        libtiff_sys::TIFFGetField(tif, tag, &mut value),
        0,
        "TIFFGetField({tag}) failed"
    );
    assert_eq!(value, expected, "tag {tag} mismatch");
}

unsafe fn assert_u32_tag(tif: *mut libtiff_sys::TIFF, tag: u32, expected: u32) {
    let mut value = 0u32;
    assert_ne!(
        libtiff_sys::TIFFGetField(tif, tag, &mut value),
        0,
        "TIFFGetField({tag}) failed"
    );
    assert_eq!(value, expected, "tag {tag} mismatch");
}

// ── Test ───────────────────────────────────────────────────────────────

/// Full roundtrip: libtiff → ZIF Parser → ZIF Writer → libtiff.
///
/// Creates a small 2‑level RGB JPEG pyramid (32×32 / 16×16 tiles plus a
/// 16×16 level), feeds it through the ZIF parser, rewrites with the ZIF
/// writer, then validates both encoded tile bytes and decoded RGBA pixels
/// match through libtiff.
#[test]
fn pyramid_roundtrips_through_libtiff() {
    // Pre‑compress each solid‑colour 16×16 tile to JPEG with mozjpeg.
    let jpeg_tiles = unsafe {
        vec![
            compress_rgb(16, 16, 255, 0, 0),     // red
            compress_rgb(16, 16, 0, 255, 0),     // green
            compress_rgb(16, 16, 0, 0, 255),     // blue
            compress_rgb(16, 16, 255, 255, 255), // white
            compress_rgb(16, 16, 128, 128, 0),   // olive (level 1)
        ]
    };

    // ── 1. Write pyramid with libtiff (raw JPEG tiles via TIFFWriteRawTile) ──
    let path_a = temp_path(NEXT_FILE.fetch_add(1, Ordering::Relaxed));
    let path_cstr = CString::new(path_a.to_string_lossy().as_bytes()).unwrap();
    let mode_w8 = CString::new("w8").unwrap();

    unsafe {
        let tif = libtiff_sys::TIFFOpen(path_cstr.as_ptr(), mode_w8.as_ptr());
        assert!(!tif.is_null(), "TIFFOpen(w8) failed");

        // --- Level 0: 32×32 RGB JPEG, 16×16 tiles, 2×2 grid (4 tiles) ---
        set_u32(tif, libtiff_sys::TIFFTAG_IMAGEWIDTH, 32);
        set_u32(tif, libtiff_sys::TIFFTAG_IMAGELENGTH, 32);
        set_u16(tif, libtiff_sys::TIFFTAG_BITSPERSAMPLE, 8);
        set_u16(tif, libtiff_sys::TIFFTAG_SAMPLESPERPIXEL, 3);
        set_u16(
            tif,
            libtiff_sys::TIFFTAG_PHOTOMETRIC,
            u16::try_from(libtiff_sys::PHOTOMETRIC_RGB).unwrap(),
        );
        set_u16(
            tif,
            libtiff_sys::TIFFTAG_COMPRESSION,
            u16::try_from(libtiff_sys::COMPRESSION_JPEG).unwrap(),
        );
        set_u16(
            tif,
            libtiff_sys::TIFFTAG_PLANARCONFIG,
            u16::try_from(libtiff_sys::PLANARCONFIG_CONTIG).unwrap(),
        );
        set_u32(tif, libtiff_sys::TIFFTAG_TILEWIDTH, 16);
        set_u32(tif, libtiff_sys::TIFFTAG_TILELENGTH, 16);
        set_u16(tif, libtiff_sys::TIFFTAG_JPEGTABLESMODE, 0);

        write_raw_tile(tif, 0, &jpeg_tiles[0]); // (0,0) red
        write_raw_tile(tif, 1, &jpeg_tiles[1]); // (1,0) green
        write_raw_tile(tif, 2, &jpeg_tiles[2]); // (0,1) blue
        write_raw_tile(tif, 3, &jpeg_tiles[3]); // (1,1) white

        // --- Level 1: 16×16 RGB JPEG, 16×16 tile, 1×1 grid (1 tile) ---
        assert_ne!(
            libtiff_sys::TIFFWriteDirectory(tif),
            0,
            "TIFFWriteDirectory failed"
        );
        set_u32(tif, libtiff_sys::TIFFTAG_IMAGEWIDTH, 16);
        set_u32(tif, libtiff_sys::TIFFTAG_IMAGELENGTH, 16);
        set_u16(tif, libtiff_sys::TIFFTAG_BITSPERSAMPLE, 8);
        set_u16(tif, libtiff_sys::TIFFTAG_SAMPLESPERPIXEL, 3);
        set_u16(
            tif,
            libtiff_sys::TIFFTAG_PHOTOMETRIC,
            u16::try_from(libtiff_sys::PHOTOMETRIC_RGB).unwrap(),
        );
        set_u16(
            tif,
            libtiff_sys::TIFFTAG_COMPRESSION,
            u16::try_from(libtiff_sys::COMPRESSION_JPEG).unwrap(),
        );
        set_u16(
            tif,
            libtiff_sys::TIFFTAG_PLANARCONFIG,
            u16::try_from(libtiff_sys::PLANARCONFIG_CONTIG).unwrap(),
        );
        set_u32(tif, libtiff_sys::TIFFTAG_TILEWIDTH, 16);
        set_u32(tif, libtiff_sys::TIFFTAG_TILELENGTH, 16);
        set_u16(tif, libtiff_sys::TIFFTAG_JPEGTABLESMODE, 0);

        write_raw_tile(tif, 0, &jpeg_tiles[4]);

        libtiff_sys::TIFFClose(tif);
    }

    let file_a = fs::read(&path_a).unwrap();

    // ── 2. Read with ZIF Parser ────────────────────────────────────────
    let mut parser = Parser::new();
    let status = parser
        .feed(DataChunk::from_start(0, file_a.clone()).unwrap())
        .unwrap();
    let ParseState::Done { image } = status else {
        panic!("ZIF Parser did not reach Done");
    };

    assert_eq!(image.level_count(), 2);
    assert_eq!(image.dimensions(), (32, 32));
    assert_eq!(image.codec(), Codec::Jpeg);
    assert_eq!(image.color_model(), ColorModel::Rgb);
    assert_eq!(image.channels(), 3);
    assert_eq!(image.kind(), ImageKind::Pyramid);

    let l0 = image.level(0).unwrap();
    assert_eq!(l0.dimensions(), (32, 32));
    assert_eq!(l0.tile_size(), (16, 16));
    assert_eq!(l0.tile_grid(), (2, 2));
    assert_eq!(l0.tile_count(), 4);

    let l1 = image.level(1).unwrap();
    assert_eq!(l1.dimensions(), (16, 16));
    assert_eq!(l1.tile_size(), (16, 16));
    assert_eq!(l1.tile_grid(), (1, 1));
    assert_eq!(l1.tile_count(), 1);

    // Extract encoded tile bytes as the ZIF parser sees them.
    let encoded_tiles: Vec<Vec<u8>> = (0..image.level_count())
        .flat_map(|li| {
            image
                .level_tiles(li)
                .unwrap()
                .map(|t| {
                    let range = t.byte_range();
                    file_a
                        [usize::try_from(range.start).unwrap()..usize::try_from(range.end).unwrap()]
                        .to_vec()
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(encoded_tiles.len(), 5);
    assert!(
        encoded_tiles.iter().all(|t| !t.is_empty()),
        "all encoded tiles must be non-empty"
    );

    // ── 3. Write with ZIF Writer ───────────────────────────────────────
    let mut builder = Writer::new()
        .codec(Codec::Jpeg)
        .color_model(ColorModel::Rgb)
        .channels(3)
        .unwrap();
    builder = builder
        .level(LevelConfig::new((32, 32), (16, 16)).unwrap())
        .level(LevelConfig::new((16, 16), (16, 16)).unwrap());
    let mut writer = builder.build().unwrap();

    let mut rewritten = Vec::new();
    let l0_positions = [(0u64, 0u64), (1, 0), (0, 1), (1, 1)];
    for idx in 0..4 {
        let (col, row) = l0_positions[idx];
        let batch = writer
            .put_tile_at_level(0, (col, row), &encoded_tiles[idx])
            .unwrap();
        apply(&mut rewritten, batch);
    }
    let batch = writer
        .put_tile_at_level(1, (0, 0), &encoded_tiles[4])
        .unwrap();
    apply(&mut rewritten, batch);

    // ── 4. Read back with libtiff and verify metadata + encoded bytes + pixels ──
    let path_b = temp_path(NEXT_FILE.fetch_add(1, Ordering::Relaxed));
    fs::write(&path_b, &rewritten).unwrap();
    let path_cstr = CString::new(path_b.to_string_lossy().as_bytes()).unwrap();
    let mode_r = CString::new("r").unwrap();

    unsafe {
        let tif = libtiff_sys::TIFFOpen(path_cstr.as_ptr(), mode_r.as_ptr());
        assert!(!tif.is_null(), "libtiff failed to open roundtripped file");

        // --- Level 0 metadata ---
        assert_u32_tag(tif, libtiff_sys::TIFFTAG_IMAGEWIDTH, 32);
        assert_u32_tag(tif, libtiff_sys::TIFFTAG_IMAGELENGTH, 32);
        assert_u16_tag(tif, libtiff_sys::TIFFTAG_BITSPERSAMPLE, 8);
        assert_u16_tag(tif, libtiff_sys::TIFFTAG_SAMPLESPERPIXEL, 3);
        assert_u16_tag(
            tif,
            libtiff_sys::TIFFTAG_PHOTOMETRIC,
            u16::try_from(libtiff_sys::PHOTOMETRIC_RGB).unwrap(),
        );
        assert_u16_tag(
            tif,
            libtiff_sys::TIFFTAG_COMPRESSION,
            u16::try_from(libtiff_sys::COMPRESSION_JPEG).unwrap(),
        );
        assert_u32_tag(tif, libtiff_sys::TIFFTAG_TILEWIDTH, 16);
        assert_u32_tag(tif, libtiff_sys::TIFFTAG_TILELENGTH, 16);
        assert_ne!(libtiff_sys::TIFFIsTiled(tif), 0);
        assert_eq!(libtiff_sys::TIFFNumberOfTiles(tif), 4);

        // Encoded tile bytes must survive the roundtrip bit‑identically.
        // NB: TIFFReadRawTile returns the compressed bitstream, not decoded pixels.
        for i in 0..4u32 {
            let mut buf = vec![0u8; encoded_tiles[i as usize].len() + 1024];
            let read = libtiff_sys::TIFFReadRawTile(
                tif,
                i,
                buf.as_mut_ptr().cast::<std::ffi::c_void>(),
                libtiff_sys::tmsize_t::try_from(buf.len()).unwrap(),
            );
            assert!(read > 0, "TIFFReadRawTile({i}) failed on level 0");
            assert_eq!(
                &buf[..usize::try_from(read).unwrap()],
                encoded_tiles[i as usize].as_slice(),
                "encoded tile {i} bytes differ"
            );
        }

        // Decoded RGBA pixels — TIFFReadRGBATile decodes the JPEG.
        let mut raster = vec![0u32; 256]; // 16×16 pixels
                                          // ABGR order on little‑endian: A<<24 | B<<16 | G<<8 | R.

        // Tile (0,0) → red
        assert_ne!(
            libtiff_sys::TIFFReadRGBATile(tif, 0, 0, raster.as_mut_ptr()),
            0,
            "TIFFReadRGBATile(0,0) failed"
        );
        let red_abgr = 0xFF00_00FFu32;
        assert!(
            raster.iter().all(|&p| p == red_abgr),
            "tile (0,0) should be solid red"
        );

        // Tile (1,0) → green
        assert_ne!(
            libtiff_sys::TIFFReadRGBATile(tif, 16, 0, raster.as_mut_ptr()),
            0,
            "TIFFReadRGBATile(16,0) failed"
        );
        let green_abgr = 0xFF00_FF00u32;
        assert!(
            raster.iter().all(|&p| p == green_abgr),
            "tile (1,0) should be solid green"
        );

        // Tile (0,1) → blue
        assert_ne!(
            libtiff_sys::TIFFReadRGBATile(tif, 0, 16, raster.as_mut_ptr()),
            0,
            "TIFFReadRGBATile(0,16) failed"
        );
        let blue_abgr = 0xFFFF_0000u32;
        assert!(
            raster.iter().all(|&p| p == blue_abgr),
            "tile (0,1) should be solid blue"
        );

        // Tile (1,1) → white
        assert_ne!(
            libtiff_sys::TIFFReadRGBATile(tif, 16, 16, raster.as_mut_ptr()),
            0,
            "TIFFReadRGBATile(16,16) failed"
        );
        let white_abgr = 0xFFFF_FFFFu32;
        assert!(
            raster.iter().all(|&p| p == white_abgr),
            "tile (1,1) should be solid white"
        );

        // --- Level 1 metadata ---
        assert_ne!(
            libtiff_sys::TIFFReadDirectory(tif),
            0,
            "TIFFReadDirectory to level 1 failed"
        );
        assert_u32_tag(tif, libtiff_sys::TIFFTAG_IMAGEWIDTH, 16);
        assert_u32_tag(tif, libtiff_sys::TIFFTAG_IMAGELENGTH, 16);
        assert_u16_tag(tif, libtiff_sys::TIFFTAG_BITSPERSAMPLE, 8);
        assert_u16_tag(tif, libtiff_sys::TIFFTAG_SAMPLESPERPIXEL, 3);
        assert_u16_tag(
            tif,
            libtiff_sys::TIFFTAG_PHOTOMETRIC,
            u16::try_from(libtiff_sys::PHOTOMETRIC_RGB).unwrap(),
        );
        assert_u16_tag(
            tif,
            libtiff_sys::TIFFTAG_COMPRESSION,
            u16::try_from(libtiff_sys::COMPRESSION_JPEG).unwrap(),
        );
        assert_u32_tag(tif, libtiff_sys::TIFFTAG_TILEWIDTH, 16);
        assert_u32_tag(tif, libtiff_sys::TIFFTAG_TILELENGTH, 16);
        assert_ne!(libtiff_sys::TIFFIsTiled(tif), 0);
        assert_eq!(libtiff_sys::TIFFNumberOfTiles(tif), 1);

        // Encoded bytes for level 1
        {
            let mut buf = vec![0u8; encoded_tiles[4].len() + 1024];
            let read = libtiff_sys::TIFFReadRawTile(
                tif,
                0,
                buf.as_mut_ptr().cast::<std::ffi::c_void>(),
                libtiff_sys::tmsize_t::try_from(buf.len()).unwrap(),
            );
            assert!(read > 0, "TIFFReadRawTile(0) failed on level 1");
            assert_eq!(
                &buf[..usize::try_from(read).unwrap()],
                encoded_tiles[4].as_slice(),
                "level 1 encoded tile bytes differ"
            );
        }

        // Decoded pixels for level 1 — olive: ABGR = 0xFF_00_80_80
        assert_ne!(
            libtiff_sys::TIFFReadRGBATile(tif, 0, 0, raster.as_mut_ptr()),
            0,
            "TIFFReadRGBATile(0,0) on level 1 failed"
        );
        let olive_abgr = 0xFF00_8080u32;
        assert!(
            raster.iter().all(|&p| p == olive_abgr),
            "level 1 tile should be solid olive"
        );

        // No more directories.
        assert_eq!(
            libtiff_sys::TIFFReadDirectory(tif),
            0,
            "unexpected extra directories"
        );

        libtiff_sys::TIFFClose(tif);
    }
}
