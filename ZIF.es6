class ZIF {

  constructor(file) {
    this.file = file;
    this.head = this.parseHead(); // A promise for the header
  }

  getTile(x, y, zoomLevel) {
    return this.getLevel(zoomLevel).then(level => level.getTile(x,y));
  }

  getLevel(zoomLevel) {
    return this.head.then(levels => levels[zoomLevel]);
  }

  parseHead() {
    return this.file.getBytes(0, ZIF.MAX_HEAD_SIZE).then(b => this.parseHeadBytes(b));
  }

  parseHeadBytes(bytes) {
    if (bytes.long(0) !== 0x08002b4949) throw new Error("invalid zif file");
    let levels = [];
    let ptr = 0x8;
    while (ptr < ZIF.MAX_HEAD_SIZE) {
      ptr = bytes.long(ptr);
      if (ptr == 0) break;
      
      let nTags = bytes.long(ptr);
      let level = new ZoomLevel(this.file);
      levels.push(level);
      let end = ptr + 8 + 20 * nTags;
      for(ptr += 8; ptr < end; ptr += 20) {
        level.set(bytes.short(ptr), bytes.long(ptr + 4), bytes.long(ptr + 12));
      }
    }
    return levels;
  }
  static MAX_HEAD_SIZE = 8192;
}

class ZoomLevel {
  constructor(file) {
    this.file = file;
    this.meta = new Map();
    this.tilesInfos = null;
  }

  getTilesInfos() {
    if (this.tilesInfos === null) {
      this.tilesInfos = Promise.all([this.getTileOffsets(), this.getTileSizes()])
                          .then(ZoomLevel.transpose);
    }
    return this.tilesInfos;
  }

  getTile(x, y) {
    return this.getTilesInfos().then(infos => this.subFile(infos[this.xy2num(x, y)]));
  }

  set(k, v1, v2) {
    return this.meta.set(k, [v1,v2]);
  }

  get(k){
    return this.meta.get(k[0])[k[1]];
  }

  static m_width    = [0x0100, 1];
  static m_height   = [0x0101, 1];
  static m_tilesize = [0x0142, 1];
  static m_count    = [0x0144, 0];
  static m_pos      = [0x0144, 1];
  static m_size     = [0x0145, 1];

  dimensions() {
    return [this.get(ZoomLevel.m_width), this.get(ZoomLevel.m_height)];
  }

  widthInTiles() {
    return Math.ceil(this.get(ZoomLevel.m_width) / this.get(ZoomLevel.m_tilesize));
  }
  heightInTiles() {
    return Math.ceil(this.get(ZoomLevel.m_height) / this.get(ZoomLevel.m_tilesize));
  }

  xy2num(x,y) {
    return x + y * this.widthInTiles();
  }

  subFile(posAndSize) {
    let [pos, size] = posAndSize;
    return this.file.getBytes(pos, pos+size)
            .then(bytes => new Blob([bytes.u8], {"type":"image/jpeg"}));
  }

  getTileOffsets() {
    const count = this.get(ZoomLevel.m_count);
    const tagval = this.get(ZoomLevel.m_pos);
    if (count === 1) return Promise.resolve([tagval]);
    return this.getUintArray(tagval, count, 8);
  }

  getTileSizes() {
    const count = this.get(ZoomLevel.m_count);
    const tagval = this.get(ZoomLevel.m_size);
    if (count < 3) {
      return Promise.resolve([tagval|0, tagval/0x100000000|0].slice(0, count));
    }
    return this.getUintArray(tagval, count, 4);
  }

  getUintArray(pos, count, bytesPerNum) {
    return this.file.getBytes(pos, pos + bytesPerNum * count)
            .then(bytes => {
                let res = new Array(bytes.length / bytesPerNum);
                for (var i = 0; i < res.length; i++)
                  res[i] = bytes.readLittleEndian(i*bytesPerNum, bytesPerNum);
                return res;
            });
  }

  static transpose(arrarr) {
    return arrarr[0].map((val, i) => arrarr.map(arr => arr[i]));
  }
}

class Bytes {
  // Little endian bytes
  constructor(buffer) {
    this.u8 = new Uint8Array(buffer);
    this.length = this.u8.length;
  }

  readLittleEndian(pos, bytes) {
    // Warning: javascript cannot store integers larger than 52 bits
    let multiplier = 1, res = 0;
    for(let i=pos; i<pos+bytes; i++) {
      res += multiplier * (this.u8[i] | 0);
      multiplier *= 0x100;
    }
    return res;
  }
  short(pos) {return this.u8[pos] | this.u8[pos + 1] << 8;}
  int(pos) {
    return this.u8[pos]           | this.u8[pos + 1] << 8 |
           this.u8[pos + 2] << 16 | this.u8[pos + 3] << 24 ;
  }
  long(pos) {return this.readLittleEndian(pos, 8);}
}

class LocalFile {
  constructor(file) {this.file = file;}
  getBytes(begin, end) {
    return new Promise((accept, reject) => {
      let r = new FileReader;
      r.onload = (evt) => accept(new Bytes(r.result));
      r.onerror = (evt) => reject(r.error);
      r.readAsArrayBuffer(this.file.slice(begin, end));
    });
  }
}
