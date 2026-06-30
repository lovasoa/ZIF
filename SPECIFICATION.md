# ZIF — Zoomable Image File Format

ZIF (Zoomable Image File) is a multi-resolution, tiled image container for
interactive panning and zooming of very large images over the Web and other
networks.

A ZIF file stores one logical image as a chain of **directories**. The
primary form is a **pyramid** of **levels**: the base level is the
full-resolution image, and each higher level is a half-resolution
down-sampling of the one below it. The same chain may instead represent a
time series or a collection of distinct images (§8). Every directory is
divided into square **tiles**, and every tile is a standalone, independently
decodable **JPEG or PNG image**. Because tiles are self-contained and
addressable by byte offset, a client fetches only the tiles it needs using
plain HTTP byte-range requests — no image server is required for basic
operation.

This document fully specifies the ZIF file format. No other document is needed
to implement a reader or writer.

The key words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** are used as in
IETF RFC 2119.

---

## 1. Conformance

ZIF has two levels:

- **Baseline** — 8-bit grayscale or RGB images, JPEG or PNG tiles, pyramids,
  focal (Z) stacks, time series, and image collections.
- **Advanced** — adds the JPEG XR and JPEG 2000 tile codecs, RGB JPEG, and
  JPEG XT, for dedicated or LAN-based use.

A reader conforms to a level if it decodes every file valid at that level. A
writer conforms to a level if it produces only files valid at that level.

Invariants that hold for **all** ZIF files:

- Little-endian byte order throughout.
- 64-bit file offsets (files may exceed 4 GB).
- All image data is **tiled** (no strips), **8-bit**, **1- or 3-channel**,
  **interleaved**.
- Tiles are self-contained codec streams.
- Tile dimensions are multiples of 16; edge tiles are clipped, not padded.
- Only the codecs in §6 are permitted. LZW, Deflate, raw/uncompressed, and the
  Aperio codes 33003 and 33005 are forbidden.

---

## 2. Primitive types

All multi-byte integers are **little-endian** (least significant byte first).
For example, `0x2B00` is stored as `00 2B`.

| Type     | Size   | Range                           |
|----------|--------|---------------------------------|
| `u8`     | 1 byte | 0 .. 255                        |
| `u16`    | 2 bytes| 0 .. 65 535                     |
| `u32`    | 4 bytes| 0 .. 4 294 967 295              |
| `u64`    | 8 bytes| 0 .. 2⁶⁴ − 1                    |

An **offset** is a `u64` byte position measured from the start of the file
(byte 0). The offset value `0` is used as a null/terminator sentinel in
chains.

---

## 3. File layout

A ZIF file begins with a fixed 16-byte **header** (§4). The header holds the
offset of the first **directory** (§5). Directories are linked in a chain:
each directory holds the offset of the next, and the last holds `0` to mark
the end. Each directory also holds offsets into a pool of **blobs** — tile
data, the tile offset/size arrays, metadata, and thumbnails — which may sit
anywhere in the file.

```
  Header --> Directory 1 --> Directory 2 --> ... --> Directory N --> 0 (end)
```

Horizontal arrows are next-directory pointers, and the chain stops at `0`.
Only the header has a fixed position (the start of the file); every other
block — directories and blobs alike — may appear anywhere and is found by
following offsets. It is RECOMMENDED that the directories be grouped together
right after the header, so the entire directory structure can be read with one
small request.

---

## 4. Header

The file begins with a 16-byte header:

| Bytes  | Type  | Field                | Required value                       |
|--------|-------|----------------------|--------------------------------------|
| 0–1    | `u16` | Byte order           | `0x4949` (ASCII `II`, little-endian) |
| 2–3    | `u16` | Version              | `0x002B`                             |
| 4–5    | `u16` | Offset size          | `0x0008` (64-bit offsets)            |
| 6–7    | `u16` | Reserved             | `0x0000`                             |
| 8–15   | `u64` | First directory offset | (file position of directory 1)     |

Equivalently, the first 8 bytes of every ZIF file are exactly:

```
49 49 2B 00 08 00 00 00
```

A reader MUST reject the file if any of the constant fields differs from the
required value above.

---

## 5. Directories

A **directory** describes one image plane: its dimensions, tile size, codec,
and the location and size of every tile. The directories are linked in a chain;
each directory is one **level** of the pyramid (or one frame/image — see §8).

### 5.1 Directory layout

| Offset | Type  | Meaning                                                 |
|--------|-------|---------------------------------------------------------|
| 0      | `u64` | number of entries in this directory (`N`)               |
| 8      | entry | Entry 0 (20 bytes).                                     |
| 28     | entry | Entry 1 (20 bytes).                                     |
| …      | …     | …                                                       |
| 8+20*N | `u64` | Offset of the next directory in the chain. `0` if last. |

The offset values in the table above are relative to the start of the directory.
Entries within a directory MUST be sorted in ascending order of their entry
code (§9.1).

### 5.2 Entry layout

Each entry is 20 bytes:

| Offset (within entry) | Type  | Meaning                                   |
|-----------------------|-------|-------------------------------------------|
| 0                     | `u16` | Entry code (§9.1).                        |
| 2                     | `u16` | Value type (§9.2).                        |
| 4                     | `u64` | `count` — number of values.               |
| 12                    | 8 B   | Value slot (see §5.3).                    |

### 5.3 The value slot (inline or by reference)

Every entry stores its value(s) either **inline** in the 8-byte slot, or
**by reference** elsewhere in the file. The rule is uniform:

- Let `total = count × (bytes per value of the given type)`.
- If `total ≤ 8`, the values are stored inline in the slot, packed
  little-endian starting at byte 12. Trailing unused bytes are ignored by
  readers and SHOULD be zero.
- If `total > 8`, the slot holds a `u64` offset (8-byte aligned) to the value
  array stored elsewhere in the file.

This single rule covers all entries. Its practical effect on tiles: a level
with one tile stores that tile's 8-byte offset inline; a level with many tiles
stores the offset array separately and the entry points to it.

---

## 6. Tiles and codecs

### 6.1 Pixel format

Tiles carry **8-bit** pixels, either **grayscale** (1 channel) or **RGB**
(3 channels), **interleaved** (channel values packed per pixel). There are no
other bit depths, no alpha, and no separate-plane storage.

### 6.2 Tiling

Each level is divided into a grid of tiles. Given image width `W`, height `H`,
tile width `Tw`, tile height `Th`:

```
tilesAcross = ceil(W / Tw)
tilesDown   = ceil(H / Th)
tileCount   = tilesAcross × tilesDown
```

- `Tw` and `Th` MUST each be a multiple of 16. Square 512 × 512 tiles are
  strongly RECOMMENDED.
- Edge tiles are **clipped** to the image boundary, not padded. A tile at
  column `c`, row `r` covers pixels
  `[c·Tw, min((c+1)·Tw, W)) × [r·Th, min((r+1)·Th, H))`.

> Clipped edges match tile-based viewers such as OpenSeadragon/DZI, OpenLayers,
> and Deck.gl. Viewers that require padded edges (Mapbox GL, Leaflet, …) are
> not directly compatible and need a tile server.

### 6.3 Tile order

Tiles are numbered in **row-major order**: left-to-right, top-to-bottom,
starting at 0. The tile at column `c`, row `r` has index:

```
i = r × tilesAcross + c
```

### 6.4 Tiles are self-contained

Each tile is a complete, independently decodable image stream in the level's
codec. Extracting the byte range `[offsets[i], offsets[i] + counts[i])` and
handing it to a standard codec decoder MUST yield the tile image, with no
reference to other tiles or to shared tables. This is what makes serverless
byte-range delivery possible.

### 6.5 Codecs

Each level uses a single codec for all its tiles, recorded as a 16-bit code:

| Code   | Codec       | Level     |
|--------|-------------|-----------|
| 7      | JPEG        | Baseline  |
| 34933  | PNG         | Baseline  |
| 34934  | JPEG XR     | Advanced  |
| 34712  | JPEG 2000   | Advanced  |

JPEG here is the legacy JFIF JPEG (ITU-T T.81 / ISO/IEC 10918-1), as commonly
deployed on the Web. JPEG 2000 uses the standard code 34712; the non-standard
Aperio codes 33003 and 33005 MUST NOT be used. Different levels in the same
file MAY use different codecs.

### 6.6 Color models

The color model is recorded as a 16-bit code:

| Code | Model        | Channels | Notes                                  |
|------|--------------|----------|----------------------------------------|
| 0    | WhiteIsZero  | 1        | Grayscale, 0 = white.                  |
| 1    | BlackIsZero  | 1        | Grayscale, 0 = black. Recommended.     |
| 2    | RGB          | 3        | Red, Green, Blue.                      |
| 6    | YCbCr        | 3        | JPEG luma/chroma.                      |

- 1-channel tiles use WhiteIsZero or BlackIsZero.
- 3-channel JPEG tiles use YCbCr or RGB. For YCbCr, chroma subsampling 4:4:4
  or 4:2:0 is permitted. For RGB, subsampling MUST be 4:4:4.
- Compatibility note: non-conforming files have been observed in the wild with
  YCbCr 4:2:2 subsampling. Readers MAY accept such files for recovery or
  faithful rewriting, but conforming writers MUST NOT create them as new ZIF
  files.
- RGB JPEG is an Advanced feature; Baseline JPEG uses YCbCr for 3-channel
  images.

### 6.7 JPEG tile packaging

Every JPEG tile is a complete JFIF stream. To keep each tile independently
decodable:

1. Quantization and Huffman tables MUST be duplicated in every tile.
2. The JFIF APPn colorspace marker MUST be duplicated in every tile.
3. The Adobe APPn colorspace marker SHOULD also be present, for decoder
   compatibility.

For dedicated/embedded use, tiles MAY use JPEG XT while still reporting codec
code 7, since JPEG XT is backwards-compatible with JPEG; decoders without JPEG
XT support simply decode the base JPEG layer.

---

## 7. Thumbnails

A thumbnail is an optional small image attached to a directory. For a pyramid
it attaches to the base directory; for other content it may attach to each
directory.

A thumbnail is stored in a **sub-directory** — a directory with exactly the
same layout as a top-level directory (§5.1). A directory references its
thumbnail via an entry with code `330 (0x014A) — Sub-directory` (§9.1), whose
value is a single `u64` directory offset (value type `dir-offset`, §9.2). The
8-byte offset is stored inline in the entry's value slot (§5.3). A directory
has at most one thumbnail.

- A thumbnail is a **solid** (non-tiled) image — a single strip/raster, not a
  tile grid.
- It MUST be JPEG or PNG compressed.
- It MUST NOT exceed 4096 × 4096 pixels; a largest side of about 1024 pixels is
  RECOMMENDED.
- For JPEG thumbnails, progressive YCbCr 4:2:0 is RECOMMENDED but not
  mandatory.

---

## 8. Pyramids, time series, and collections

The chain of directories (linked by each directory's next-directory offset) is
interpreted by how the image dimensions relate:

| Relationship between consecutive directories      | Interpretation                |
|---------------------------------------------------|-------------------------------|
| Each is ≈ half the previous (rounded up).         | Multi-resolution pyramid.     |
| All have identical dimensions.                    | Time series (frames).         |
| Otherwise.                                        | Collection of distinct images.|

### 8.1 Pyramid levels

Directory 1 is the **base level** (full resolution, `W × H`). Each subsequent
level halves both dimensions, rounded up:

```
W_{k+1} = ceil(W_k / 2)
H_{k+1} = ceil(H_k / 2)
```

Each lower level's pixels are the previous level down-sampled by exactly two
in each axis, left/top aligned. A writer SHOULD continue halving until both
dimensions are ≤ the tile size (so the top level is a single tile) and MAY
continue to 1 × 1. A reader follows the chain until the next-directory offset
is `0`; it MUST NOT assume a fixed number of levels.

### 8.2 Z-dimension (focal stacks)

An image MAY carry a Z (depth) axis representing a focal stack. Without
overriding metadata, the Z axis always denotes **focal depth** — never time,
exposure, or any other axis (time is expressed by the directory chain). The
encoding of the Z axis is implementation-defined; this specification fixes only
its semantics.

---

## 9. Binary encoding reference

This section lists the numeric codes used on disk. The header and
directory/entry structure are defined in §4–§5; the tables below give the
specific codes a writer must emit and a reader must interpret.

### 9.1 Entry codes

Entries MUST be sorted within a directory by ascending code. "Req." marks
entries required in every image directory.

| Code (hex)    | Records          | Type (typical)      | Req. | Value / constraint                          |
|---------------|------------------|---------------------|------|---------------------------------------------|
| 254 (0x00FE)  | Flags            | `u32`               | no   | Bitmask; bit 0 = reduced-resolution level (informational). |
| 256 (0x0100)  | Image width      | `u16`/`u32`         | yes  | Level width in pixels.                      |
| 257 (0x0101)  | Image height     | `u16`/`u32`         | yes  | Level height in pixels.                     |
| 258 (0x0102)  | Bit depth        | `u16` array         | yes  | One value `8`, or one `8` per channel.      |
| 259 (0x0103)  | Codec            | `u16`               | yes  | §6.5.                                       |
| 262 (0x0106)  | Color model      | `u16`               | yes  | §6.6.                                       |
| 277 (0x0115)  | Channels         | `u16`               | yes  | 1 (grayscale) or 3 (RGB).                   |
| 284 (0x011C)  | Interleave       | `u16`               | yes  | Always 1 (interleaved).                     |
| 322 (0x0142)  | Tile width       | `u16`/`u32`         | yes  | Multiple of 16.                             |
| 323 (0x0143)  | Tile height      | `u16`/`u32`         | yes  | Multiple of 16.                             |
| 324 (0x0144)  | Tile offsets     | `u32`/`u64` array   | yes  | `tileCount` file offsets, one per tile (§6.3). |
| 325 (0x0145)  | Tile byte counts | `u32`/`u64` array   | yes  | `tileCount` sizes in bytes, one per tile.   |
| 330 (0x014A)  | Sub-directory    | dir-offset          | no   | Offset to a thumbnail sub-directory (§7).    |
| 530 (0x0212)  | YCbCr subsampling| `u16` array         | no   | Two values: horizontal and vertical subsampling. Valid values are `1,1` (4:4:4) or `2,2` (4:2:0). SHOULD be present when color model is YCbCr. |
| 271 (0x010F)  | Make             | ASCII               | no   | Device manufacturer (§10.2).                |
| 272 (0x0110)  | Model            | ASCII               | no   | Device model (§10.2).                       |
| 305 (0x0131)  | Software         | ASCII               | no   | Producing software (§10.2).                 |
| 34665 (0x8769)| EXIF metadata    | dir-offset          | no   | Offset to an EXIF sub-directory.            |
| 51159 (0xC7D7)| ZIF metadata     | opaque bytes        | no   | Implementation-defined metadata blob.       |
| 51160 (0xC7D8)| ZIF annotations  | opaque bytes        | no   | Implementation-defined annotation blob.     |

The *Tile offsets* and *Tile byte counts* arrays both have length `tileCount`
and are indexed identically in row-major order (§6.3). Tile `i` occupies the
byte range `[offsets[i], offsets[i] + counts[i])`. Writers MAY store tile
offsets as either `u32` or `u64`; readers MUST accept both. Writers MAY store
tile byte counts as either `u32` or `u64`; readers MUST accept both, but MUST
reject a count that does not fit in the reader's supported memory model. Writers
SHOULD use the smallest integer type that can represent all values in the array.

For three-channel images, the *Bit depth* entry MAY contain either a single
value `8` applying to all channels, or three values `8, 8, 8`, one per channel.
For one-channel images, it MUST contain one value `8`.

An entry with a code not listed here MUST be ignored by readers that do not
understand it, provided it does not contradict a required entry.

### 9.2 Value types

The value type is a 16-bit code in each entry. The types used by ZIF:

| Code | Type        | Size   | Notes                                            |
|------|-------------|--------|--------------------------------------------------|
| 1    | byte        | 1      | Generic/opaque byte.                             |
| 2    | ASCII       | 1      | NUL-terminated string; `count` includes the NUL. |
| 3    | `u16`       | 2      |                                                  |
| 4    | `u32`       | 4      |                                                  |
| 16   | `u64`       | 8      | Used for offsets and large counts.               |
| 18   | dir-offset  | 8      | A `u64` offset pointing to a directory.          |

The inline/reference rule (§5.3) applies regardless of type. Readers SHOULD
apply the same rule to any other type code they encounter.

---

## 10. Metadata and provenance

### 10.1 Metadata

A ZIF file MAY carry:

- **EXIF metadata** (entry 34665): a directory offset to EXIF content.
- **ZIF metadata** (entry 51159): an opaque, implementation-defined blob.
- **ZIF annotations** (entry 51160): an opaque, implementation-defined blob.

The structure of the ZIF metadata and annotations blobs is not fixed by this
specification. EXIF content follows the EXIF standard.

### 10.2 Device and software identification

The *Make* (271), *Model* (272), and *Software* (305) entries are ASCII
strings. Registered integer values (when used as a numeric code rather than a
free string):

| Entry    | 0            | 1–15                          | 16–23 / 16–31                  |
|----------|--------------|-------------------------------|--------------------------------|
| Make     | unknown      | Objective Pathology Services  | 2 = Zoomify; 3 = Huron Digital Pathology |
| Model    | unknown/software | Objective Pathology Services (1–15) | Huron Digital Pathology (16–31) |
| Software | unknown/hardware | Objective Pathology Services (1–15) | Zoomify (16–23); Huron Digital Pathology (24–39) |

---

## 11. Reading a ZIF file

A minimal Baseline reader:

1. Read the 16-byte header; validate the constants; take the first directory
   offset.
2. Follow next-directory pointers, reading each directory's entry count and
   entries.
3. For each directory, read image width/height, tile size, codec, color model,
   channels, and the tile-offset and tile-byte-count arrays (resolving each via
   §5.3).
4. Compute `tilesAcross`, `tilesDown`, `tileCount` (§6.2).
5. To show the tile at column `c`, row `r` of a level: compute
   `i = r × tilesAcross + c`, fetch bytes
   `[offsets[i], offsets[i] + counts[i])`, and decode them with the codec.
6. Interpret the chain per §8 (pyramid / time series / collection).

Because directory data can be grouped at the start of the file and each tile is
fetched by an independent byte range, the whole format suits HTTP `Range`
delivery without an image server.

---

## Appendix A — Heritage note (non-normative)

The numeric constants in this specification — the header magic, the directory
and entry byte layouts, the entry codes in §9.1, and the value-type codes in
§9.2 — coincide, byte for byte, with those of the BigTIFF format. ZIF was
designed as a restricted profile of that container so that existing imaging
libraries can open ZIF files directly (a ZIF file can often be read by renaming
its extension to `.tif`). This shared lineage explains why the entry codes
look the way they do, but it is not relied upon anywhere above: everything
required to read or write ZIF is stated in this document.

*End of specification.*
