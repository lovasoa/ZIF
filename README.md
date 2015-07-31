# ZIF
zoomify ZIF (ZoomifyImageFormat) file format documentation and tools.

The ZIF file format is used by zoomify as a single-file zoomable image format.
A zif file represents a single image. It contains JPEG *tiles*, little square images that represent a part of the image at a given zoom level, as well as meta-information about how the tiles are organized. [More about the concept behind zoomify](https://msdn.microsoft.com/en-us/library/cc645050%28VS.95%29.aspx).

![AN image and its tiles](http://www.zoomify.com/downloads/screenshots/tiledTiered.jpg)

# ZIF file format documentation
I have partially reverse-engineered the format. Here is what I found.

### Data types
All numbers are stored in [**little endian**](https://en.wikipedia.org/wiki/Endianness) (the number `0xABCD` is sored as `0xCD 0xAB`).

Term         | Signification
-------------|---------------
long         | 8 bytes unsigned int (uint64)
int          | 4 bytes unsigned int (uint32)
short        | 2 bytes unsigned int (uint16)
pointer      | a *long* representing an offset in bytes from the beginning of the file

## Header

### Magic bytes
The first 16 bytes of the file are always
```
49 49 2B 00 08 00 00 00 10 00 00 00 00 00 00 00
```

### Metadata
Meta-data start at offset `0x10` in the file.
There is a set of metadata for every zoom level (tier).

#### Zoomlevel metadata
Each set of metadata starts with a long **pointer** to a long number representing the number of tags (single metadata) in the zoom level.

Each tag is 20 bytes long.

Offset (from the start of the tag) | Length (in bytes) | Data
-----------------------------------|-------------------|------------------------------
0                                  | 2 (short)         | Magic number identifying the tag type
2                                  | 2                 | Unknown (but not null)
4                                  | 8  (long)         | **value 1**
12                                 | 8  (long)         | **value 2**

##### Tag types
Magic number in decimal | Magic bytes | Signification of **value 1** | Signification of **value 2**
-----|---|---|---
256  |`0x00 0x01`| ?                        | Image width at this zoomlevel
257  |`0x01 0x01`| ?                        | Image height at this zoomlevel
322  |`0x42 0x01`| ?                        | Tile width
323  |`0x43 0x01`| ?                        | Tile height
324  |`0x44 0x01`| Number of tiles          | Pointer to the *tile offsets index*


