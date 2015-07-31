# ZIF
zoomify ZIF (ZoomifyImageFormat) file format documantation and tools

## ZIF file format documentation
I have partially reverse-engineered the format. Here is what I found.

## Header

### Magic bytes
The first 16 bytes of the file are always
```
49 49 2B 00 08 00 00 00 10 00 00 00 00 00 00 00
```
