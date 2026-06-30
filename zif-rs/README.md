# zif

`zif` is a Rust library for reading and writing [ZIF](../SPECIFICATION.md), a tiled, multi-resolution image format for very large zoomable images.

ZIF stores image metadata in a BigTIFF-compatible container and stores each tile as an independent JPEG or PNG byte stream. This makes it a good fit for local files, HTTP range requests, object storage, and interactive viewers that only need a small part of a huge image at a time.

The core crate is Sans-IO: it parses metadata, plans byte-range requests, exposes tile locations, and writes ZIF structures without choosing a filesystem, HTTP client, async runtime, or image decoder for you.

## Highlights

- zero dependencies in the core library
- Sans-IO reader and writer
- efficient byte-range access for huge files
- no JPEG or PNG decoding in core
- ergonomic tile iteration for viewports and zoom levels
- optional `std`, `tokio`, and `reqwest` IO adapters
- writer updates that keep the file valid at every step

## Quick Start

Read a remote ZIF file and inspect its dimensions:

```rust
let mut io = zif::reqwest::HttpRangeReader::new("https://example.com/slide.zif");
let mut reader = zif::Reader::new();
let mut chunk = zif::Chunk::default();

while let zif::ReadStatus::NeedMore(req) = reader.advance(chunk)? {
    chunk = io.fetch(req).await?;
}

let zif = reader.zif()?;

println!(
    "The image is {} x {}, with {} levels",
    zif.width(),
    zif.height(),
    zif.level_count(),
);
```

Fetch the encoded tiles intersecting a viewport:

```rust
let mut io = zif::reqwest::HttpRangeReader::new("https://example.com/slide.zif");
let level = 2;

// Region is (x_range, y_range), in pixels at this level.
let region = (10_000..20_000, 15_000..25_000);

for tile in zif.get_cropped_level_tiles(level, region)? {
    println!(
        "tile ({}, {}) at {:?}, size {:?}",
        tile.col(),
        tile.row(),
        tile.position(),
        tile.size(),
    );

    let encoded_tile = io.fetch(tile.req()).await?;
    write_to_file(encoded_tile.bytes())?;
}
```

The bytes returned for a tile are the original encoded JPEG or PNG stream. Decode them with the image library appropriate for your application.

## Installation

Core only:

```toml
[dependencies]
zif = "..."
```

With synchronous filesystem helpers:

```toml
[dependencies]
zif = { version = "...", features = ["std"] }
```

With Tokio filesystem helpers:

```toml
[dependencies]
zif = { version = "...", features = ["tokio"] }
```

With Reqwest HTTP range helpers:

```toml
[dependencies]
zif = { version = "...", features = ["reqwest"] }
```

With async filesystem and HTTP helpers:

```toml
[dependencies]
zif = { version = "...", features = ["tokio", "reqwest"] }
```

Feature flags:

- `std`: standard-library filesystem readers and writers
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

The reader accepts coherent byte chunks and returns the next byte-range request until metadata is complete.

```rust
let mut io = zif::std::FileRangeReader::open("slide.zif")?;
let mut reader = zif::Reader::new();
let mut chunk = zif::Chunk::default();

while let zif::ReadStatus::NeedMore(req) = reader.advance(chunk)? {
    chunk = io.fetch(req)?;
}

let zif = reader.zif()?;
```

`advance` accepts any valid chunk, not only the exact range it requested. This is useful for HTTP servers that return a larger range than requested or return the whole file.

```rust
let response = client.fetch(req).await?;
let chunk = zif::Chunk::new(response.content_range, response.body)?;
reader.advance(chunk)?;
```

If a server returns the full file:

```rust
let chunk = zif::Chunk::from_start(0, body)?;
reader.advance(chunk)?;
```

`Chunk` ties a byte range to the bytes for that range. A chunk cannot be constructed if the range length and byte length disagree.

## Inspecting Metadata

Common metadata is available directly from `Zif`:

```rust
let zif = reader.zif()?;

println!("dimensions: {:?}", zif.dimensions());
println!("width: {}", zif.width());
println!("height: {}", zif.height());
println!("levels: {}", zif.level_count());
println!("codec: {:?}", zif.codec());
println!("color model: {:?}", zif.color_model());
println!("channels: {}", zif.channels());
```

Level metadata is available from `Level`:

```rust
let level = zif.level(2)?;

println!("level dimensions: {:?}", level.dimensions());
println!("tile size: {:?}", level.tile_size());
println!("tile grid: {:?}", level.tile_grid());
println!("tile count: {}", level.tile_count());
```

## Iterating Tiles

All tiles at a level:

```rust
for tile in zif.get_level_tiles(2)? {
    println!(
        "level {} tile {} at ({}, {}) uses bytes {:?}",
        tile.level(),
        tile.index(),
        tile.col(),
        tile.row(),
        tile.bytes(),
    );
}
```

Tiles intersecting a viewport:

```rust
let mut io = zif::reqwest::HttpRangeReader::new("https://example.com/slide.zif");

// Region is (x_range, y_range), in pixels at level 2.
let visible = (10_000..20_000, 15_000..25_000);

for tile in zif.get_cropped_level_tiles(2, visible)? {
    let encoded = io.fetch(tile.req()).await?;
    renderer.submit_encoded_tile(tile.position(), tile.size(), encoded.bytes())?;
}
```

Tile values are lightweight metadata objects:

```rust
impl Tile {
    pub fn level(&self) -> usize;
    pub fn index(&self) -> u64;

    pub fn col(&self) -> u64;
    pub fn row(&self) -> u64;

    pub fn x(&self) -> u64;
    pub fn y(&self) -> u64;
    pub fn width(&self) -> u64;
    pub fn height(&self) -> u64;

    pub fn position(&self) -> (u64, u64);
    pub fn size(&self) -> (u64, u64);

    pub fn bytes(&self) -> std::ops::Range<u64>;
    pub fn req(&self) -> zif::Request;
    pub fn codec(&self) -> zif::Codec;
}
```

Use `tile.bytes()` when you only need the raw byte range. Use `tile.req()` when passing the request to an IO adapter.

## IO Adapters

The core API works with any backend that can fetch byte ranges and return `Chunk` values. Built-in adapters cover common cases:

```rust
let mut io = zif::std::FileRangeReader::open("slide.zif")?;
let mut io = zif::tokio::FileRangeReader::open("slide.zif").await?;
let mut io = zif::reqwest::HttpRangeReader::new("https://example.com/slide.zif");
```

Custom backends are straightforward because the reader only needs `Request` in and `Chunk` out.

```rust
while let zif::ReadStatus::NeedMore(req) = reader.advance(chunk)? {
    let bytes = object_store.get_range(req.range()).await?;
    chunk = zif::Chunk::new(req.range(), bytes)?;
}
```

## Writing

The writer emits write operations for the caller to apply. It does not own a file handle and does not choose an IO backend.

Create a one-level image:

```rust
let mut io = zif::tokio::FileRangeWriter::create("slide.zif").await?;

let mut writer = zif::Writer::new()
    .dimensions((100_000, 80_000))
    .tile_size((512, 512))?
    .codec(zif::Codec::Jpeg)
    .color_model(zif::ColorModel::YCbCr)
    .channels(3)?
    .build()?;

let ops = writer.put_tile((0, 0), encoded_jpeg)?;
io.apply(ops).await?;
```

Create a pyramid explicitly:

```rust
let mut io = zif::tokio::FileRangeWriter::create("slide.zif").await?;

let mut writer = zif::Writer::new()
    .level(zif::LevelSpec::new((100_000, 80_000), (512, 512))?)
    .level(zif::LevelSpec::new((50_000, 40_000), (512, 512))?)
    .level(zif::LevelSpec::new((25_000, 20_000), (512, 512))?)
    .codec(zif::Codec::Jpeg)
    .color_model(zif::ColorModel::YCbCr)
    .channels(3)?
    .build()?;

let ops = writer.put_tile_at_level(1, (3, 4), encoded_jpeg)?;
io.apply(ops).await?;
```

Update dimensions and continue adding tiles:

```rust
let ops = writer.set_dimensions((120_000, 90_000))?;
io.apply(ops).await?;

let ops = writer.put_tile((12, 8), encoded_jpeg)?;
io.apply(ops).await?;
```

Writer updates use an append-first, pointer-last layout. Existing valid structures remain in place until replacement structures and tile bytes have been written. The final pointer update switches the file to the new valid layout.

This makes repetitive updates practical: adding or replacing a tile does not require rewriting existing tile payloads.

## Error Handling

Core errors are separate from IO adapter errors. Malformed files, invalid coordinates, arithmetic overflow, unsupported features, and incoherent chunks are reported as typed `zif::Error` values.

```rust
if let Err(err) = reader.advance(chunk) {
    match err {
        zif::Error::MalformedFile(err) => reject_file(err),
        zif::Error::InvalidInput(err) => report_bug(err),
        err => return Err(err.into()),
    }
}
```

The reader rejects structural violations such as invalid header constants, unsorted directory entries, missing required tags, invalid tile dimensions, tile count mismatches, and overflowing byte ranges. Unknown tags are ignored unless they contradict required ZIF metadata.

## Format Compatibility

ZIF is BigTIFF-compatible at the container level. A ZIF file can often be opened by TIFF tools that support tiled BigTIFF with JPEG or PNG compression.

The format is still intentionally narrower than general TIFF. ZIF requires little-endian byte order, 64-bit offsets, tiled image data, 8-bit grayscale or RGB pixels, and self-contained JPEG or PNG tiles for baseline files.

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
