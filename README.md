# ZIF
zoomify ZIF (ZoomifyImageFormat) file format documentation and tools.

The ZIF file format is used by zoomify as a single-file zoomable image format.
A zif file represents a single image. It contains JPEG *tiles*, little square images that represent a part of the image at a given zoom level, as well as meta-information about how the tiles are organized. [More about the concept behind zoomify](https://msdn.microsoft.com/en-us/library/cc645050%28VS.95%29.aspx).

![AN image and its tiles](http://www.zoomify.com/downloads/screenshots/tiledTiered.jpg)

## ZIF file format documentation
I have partially reverse-engineered the format. Here is what I found.

## Header

### Magic bytes
The first 16 bytes of the file are always
```
49 49 2B 00 08 00 00 00 10 00 00 00 00 00 00 00
```

