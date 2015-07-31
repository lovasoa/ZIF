# ZIF
zoomify ZIF (ZoomifyImageFormat) file format documentation and tools.

The ZIF file format is used by zoomify as a single-file zoomable image format.
A zif file represents a single image. It contains JPEG *tiles*, little square images that represent a part of the image at a given zoom level, as well as meta-information about how the tiles are organized. [More about the concept behind zoomify](https://msdn.microsoft.com/en-us/library/cc645050%28VS.95%29.aspx).

![AN image and its tiles](http://www.zoomify.com/downloads/screenshots/tiledTiered.jpg)

# ZIF file format documentation
I have partially reverse-engineered the format. Here is what I found.

### Vocabulary
Term         | Signification
-------------|---------------
long         | 8 bytes unsigned int (uint64)
int          | 4 bytes unsigned int (uint32)
short        | 2 bytes unsigned int (uint16)
pointer      | a *long* representing an offset from the beginning of the file

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
0                                  | 2                 | Tag name
2                                  | 2                 | Unknown (but not null)
4                                  | 8                 | **value 1**
12                                 | 8                 | **value 2**
