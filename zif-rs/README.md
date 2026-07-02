# zif-tiff

`zif-tiff` is a Rust library for reading and writing [ZIF](../SPECIFICATION.md), a tiled, multi-resolution image format for very large zoomable images. ZIF is a constrained subset of BigTIFF: every ZIF file is a valid TIFF file, but most TIFF files are not valid ZIF files.

ZIF stores image metadata in a BigTIFF-compatible container and stores each tile as an independent JPEG or PNG byte stream. This makes it a good fit for local files, HTTP range requests, object storage, and interactive viewers that only need a small part of a huge image at a time.

The core crate is Sans-IO: it parses metadata, plans byte-range requests, exposes tile locations, and writes ZIF structures without choosing a filesystem, HTTP client, async runtime, or image decoder for you.

## Highlights

- zero dependencies in the core library
- Sans-IO reader and writer
- efficient byte-range access for huge files
- no JPEG or PNG decoding in core
- ergonomic tile iteration for viewports and zoom levels
- default `std` IO adapters for files and any `Read + Seek` / `Write + Seek` object
- optional `tokio` and `reqwest` IO adapters
- writer updates that keep the file valid at every step

## Quick Start

Read a ZIF file and inspect its dimensions:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use zif_tiff::{sample_zif_bytes, std::RangeReader};

    let zif = RangeReader::from(sample_zif_bytes()).read_zif()?;
    println!("{} x {}, {} levels", zif.width(), zif.height(), zif.level_count());

    Ok(())
}
```

Use the Sans-IO reader when you want explicit control over range requests, for example to fetch the encoded tiles intersecting a viewport:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use zif_tiff::{sample_zif_bytes, Chunk, ReadStatus, Reader, std::RangeReader};

    let mut io = RangeReader::from(sample_zif_bytes());
    let mut reader = Reader::new();
    let mut chunk = Chunk::default();

    let zif = loop {
        match reader.advance(chunk)? {
            ReadStatus::Need { req, .. } => chunk = io.fetch(req)?,
            ReadStatus::Done { zif } => break zif.as_zif().clone(),
        }
    };

    for tile in zif.get_cropped_level_tiles(0, (0..20, 0..20))? {
        let encoded_tile = io.fetch(tile.req())?;
        println!("tile {:?}, {} bytes", tile.position(), encoded_tile.bytes().len());
    }

    Ok(())
}
```

The bytes returned for a tile are the original encoded JPEG or PNG stream. Decode them with the image library appropriate for your application.

## Installation

Default features, including synchronous filesystem helpers:

```toml
[dependencies]
zif-tiff = "..."
```

Core/no-std only:

```toml
[dependencies]
zif-tiff = { version = "...", default-features = false }
```

With Tokio filesystem helpers:

```toml
[dependencies]
zif-tiff = { version = "...", features = ["tokio"] }
```

With Reqwest HTTP range helpers:

```toml
[dependencies]
zif-tiff = { version = "...", features = ["reqwest"] }
```

With async filesystem and HTTP helpers:

```toml
[dependencies]
zif-tiff = { version = "...", features = ["tokio", "reqwest"] }
```

Feature flags:

- `std`: standard-library filesystem readers and writers, enabled by default
- `tokio`: Tokio-based filesystem readers and writers
- `reqwest`: Reqwest-based HTTP range readers

## What ZIF Contains

A ZIF file represents one logical image as a chain of image directories. In the common pyramid layout:

- level 0 is the full-resolution image
- level 1 is approximately half resolution
- level 2 is approximately quarter resolution
- later levels continue down the pyramid

Each level is split into tiles. Each tile is independently compressed and can be fetched by byte range without reading or decoding the rest of the file.

The reader exposes:

- image dimensions
- level dimensions
- tile size and tile grid
- codec and color model metadata
- byte ranges for encoded tiles
- optional metadata and annotation ranges

The reader does not expose decoded pixels.

## Reading

For normal reads, use an IO adapter to get a `Zif` object directly.

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use zif_tiff::{sample_zif_bytes, std::RangeReader};

    let zif = RangeReader::from(sample_zif_bytes()).read_zif()?;

    println!("dimensions: {:?}", zif.dimensions());
    println!("codec: {:?}", zif.codec());
    println!("color model: {:?}", zif.color_model());

    Ok(())
}
```

The same helper works with any `Read + Seek` object:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = zif_tiff::sample_zif_bytes();
    let cursor = std::io::Cursor::new(bytes);
    let zif = zif_tiff::std::read_zif(cursor)?;

    assert_eq!(zif.dimensions(), (40, 40));

    Ok(())
}
```

Optional adapters provide equivalent helpers for async files and HTTP:

```text
let zif = zif_tiff::tokio::read_zif(file).await?;
let zif = zif_tiff::reqwest::read_zif("https://example.com/slide.zif").await?;
```

## Iterating Tiles

All tiles at a level:

```rust
fn main() -> zif_tiff::Result<()> {
    let zif = zif_tiff::sample::zif();

    for tile in zif.get_level_tiles(0)? {
        println!("tile {} at {:?} uses bytes {:?}", tile.index(), tile.position(), tile.bytes());
    }

    Ok(())
}
```

Tiles intersecting a viewport:

```rust
fn main() -> zif_tiff::Result<()> {
    let zif = zif_tiff::sample::zif();

    for tile in zif.get_cropped_level_tiles(0, (0..20, 0..20))? {
        println!("visible tile {:?}, size {:?}", tile.position(), tile.size());
    }

    Ok(())
}
```

Tile values are lightweight metadata objects:

```text
impl Tile {
    pub fn level(&self) -> usize;
    pub fn index(&self) -> u64;
    pub fn col(&self) -> u64;
    pub fn row(&self) -> u64;
    pub fn position(&self) -> (u64, u64);
    pub fn size(&self) -> (u64, u64);
    pub fn bytes(&self) -> std::ops::Range<u64>;
    pub fn req(&self) -> zif_tiff::Request;
    pub fn codec(&self) -> zif_tiff::Codec;
}
```

Use `tile.bytes()` when you only need the raw byte range. Use `tile.req()` when passing the request to an IO adapter.

## IO Adapters

The core API works with any backend that can fetch byte ranges and return `Chunk` values. The `std` adapter works with files and with any `Read + Seek` object:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use zif_tiff::{sample_zif_bytes, std::RangeReader};

    let mut io = RangeReader::from(sample_zif_bytes());
    let chunk = io.fetch(zif_tiff::Request::new(0..16)?)?;

    assert_eq!(chunk.range(), 0..16);

    Ok(())
}
```

Optional adapters cover common filesystem and HTTP cases:

```text
let mut file_io = zif_tiff::std::FileRangeReader::open("slide.zif")?;
let mut tokio_io = zif_tiff::tokio::FileRangeReader::open("slide.zif").await?;
let http_io = zif_tiff::reqwest::HttpRangeReader::new("https://example.com/slide.zif");
```

## Low-Level Sans-IO

Use the Sans-IO `Reader` directly when integrating with a custom storage layer or when you need exact control over range requests and response chunks. `advance` accepts any valid chunk, not only the exact range it requested. This is useful for HTTP servers that return a larger range than requested or return the whole file. `Chunk` ties a byte range to the bytes for that range and rejects incoherent range/length pairs.

Custom backends are straightforward because the reader only needs `Request` in and `Chunk` out.

```text
while let zif_tiff::ReadStatus::Need { req, .. } = reader.advance(chunk)? {
    let bytes = object_store.get_range(req.range()).await?;
    chunk = zif_tiff::Chunk::new(req.range(), bytes)?;
}
```

## Writing

The writer emits write operations for the caller to apply. It does not own a file handle and does not choose an IO backend.

Create a one-level image:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use zif_tiff::std::RangeWriter;

    let mut file = RangeWriter::from(Vec::new());
    let mut writer = zif_tiff::Writer::new()
        .dimensions((100_000, 80_000))
        .tile_size((512, 512))?
        .codec(zif_tiff::Codec::Jpeg)
        .color_model(zif_tiff::ColorModel::YCbCr)
        .channels(3)?
        .build()?;

    file.apply(writer.put_tile((0, 0), b"encoded-jpeg")?)?;
    let bytes = file.into_inner().into_inner();

    println!("wrote {} bytes", bytes.len());

    Ok(())
}
```

Create a conformant pyramid automatically:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use zif_tiff::std::RangeWriter;

    let mut file = RangeWriter::from(Vec::new());
    let mut writer = zif_tiff::Writer::new()
        .dimensions((100_000, 80_000))
        .tile_size((512, 512))?
        .pyramid()
        .codec(zif_tiff::Codec::Jpeg)
        .color_model(zif_tiff::ColorModel::YCbCr)
        .channels(3)?
        .build()?;

    file.apply(writer.put_tile_at_level(1, (3, 4), b"encoded-jpeg")?)?;

    Ok(())
}
```

Use `.pyramid()` to build levels down to a single tile, or `.pyramid_to_1x1()` to continue until the final level is exactly 1 x 1 pixels.

Update dimensions and continue adding tiles:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use zif_tiff::std::RangeWriter;

    let mut file = RangeWriter::from(Vec::new());
    let mut writer = zif_tiff::Writer::new()
        .dimensions((100_000, 80_000))
        .tile_size((512, 512))?
        .codec(zif_tiff::Codec::Jpeg)
        .color_model(zif_tiff::ColorModel::YCbCr)
        .channels(3)?
        .build()?;

    file.apply(writer.set_dimensions(0, (120_000, 90_000))?)?;
    file.apply(writer.put_tile((12, 8), b"encoded-jpeg")?)?;

    Ok(())
}
```

Writer updates use an append-first, pointer-last layout. Existing valid structures remain in place until replacement structures and tile bytes have been written. The final pointer update switches the file to the new valid layout.

This makes repetitive updates practical: adding or replacing a tile does not require rewriting existing tile payloads.

## Error Handling

Core errors are separate from IO adapter errors. Malformed files, invalid coordinates, arithmetic overflow, unsupported features, and incoherent chunks are reported as typed `zif_tiff::Error` values.

```rust
fn handle(reader: &mut zif_tiff::Reader, chunk: zif_tiff::Chunk) -> zif_tiff::Result<()> {
    match reader.advance(chunk) {
        Ok(_) => Ok(()),
        Err(zif_tiff::Error::MalformedFile(msg)) => {
            eprintln!("rejecting malformed file: {msg}");
            Ok(())
        }
        Err(err) => Err(err),
    }
}
```

The reader rejects structural violations such as invalid header constants, unsorted directory entries, missing required tags, invalid tile dimensions, tile count mismatches, and overflowing byte ranges. Unknown tags are ignored unless they contradict required ZIF metadata.

## Format Compatibility

ZIF is BigTIFF-compatible at the container level. A ZIF file can often be opened by TIFF tools that support tiled BigTIFF with JPEG or PNG compression.

The format is intentionally narrower than general TIFF. ZIF requires little-endian byte order, 64-bit offsets, tiled image data, 8-bit grayscale or RGB pixels, and self-contained JPEG or PNG tiles for baseline files.

See [SPECIFICATION.md](SPECIFICATION.md) for the complete binary format.

## Testing And Validation

The crate is designed to be tested at the format boundary:

- parse hand-built fixtures and malformed files
- round-trip files through the writer and reader
- verify generated files with TIFF tooling where available
- decode extracted JPEG and PNG tiles in tests
- exercise fragmented, oversized, duplicate, and non-exact range responses
- check huge-image arithmetic and edge-tile clipping

External crates are appropriate in tests and optional IO adapters. They are not required by the core reader, writer, and metadata model.

## License

This project is licensed under the terms in [LICENSE](LICENSE).
