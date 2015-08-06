"use strict";

var _slicedToArray = (function () { function sliceIterator(arr, i) { var _arr = []; var _n = true; var _d = false; var _e = undefined; try { for (var _i = arr[Symbol.iterator](), _s; !(_n = (_s = _i.next()).done); _n = true) { _arr.push(_s.value); if (i && _arr.length === i) break; } } catch (err) { _d = true; _e = err; } finally { try { if (!_n && _i["return"]) _i["return"](); } finally { if (_d) throw _e; } } return _arr; } return function (arr, i) { if (Array.isArray(arr)) { return arr; } else if (Symbol.iterator in Object(arr)) { return sliceIterator(arr, i); } else { throw new TypeError("Invalid attempt to destructure non-iterable instance"); } }; })();

var _createClass = (function () { function defineProperties(target, props) { for (var i = 0; i < props.length; i++) { var descriptor = props[i]; descriptor.enumerable = descriptor.enumerable || false; descriptor.configurable = true; if ("value" in descriptor) descriptor.writable = true; Object.defineProperty(target, descriptor.key, descriptor); } } return function (Constructor, protoProps, staticProps) { if (protoProps) defineProperties(Constructor.prototype, protoProps); if (staticProps) defineProperties(Constructor, staticProps); return Constructor; }; })();

function _classCallCheck(instance, Constructor) { if (!(instance instanceof Constructor)) { throw new TypeError("Cannot call a class as a function"); } }

var ZIF = (function () {
  function ZIF(file) {
    _classCallCheck(this, ZIF);

    this.file = file;
    this.head = this.parseHead(); // A promise for the header
  }

  _createClass(ZIF, [{
    key: "getTile",
    value: function getTile(x, y, zoomLevel) {
      return this.getLevel(zoomLevel).then(function (level) {
        return level.getTile(x, y);
      });
    }
  }, {
    key: "getLevel",
    value: function getLevel(zoomLevel) {
      return this.head.then(function (levels) {
        return levels[zoomLevel];
      });
    }
  }, {
    key: "parseHead",
    value: function parseHead() {
      var _this = this;

      return this.file.getBytes(0, ZIF.MAX_HEAD_SIZE).then(function (b) {
        return _this.parseHeadBytes(b);
      });
    }
  }, {
    key: "parseHeadBytes",
    value: function parseHeadBytes(bytes) {
      if (bytes.long(0) !== 0x08002b4949) throw new Error("invalid zif file");
      var levels = [];
      var ptr = 0x8;
      while (ptr > 0 && ptr < ZIF.MAX_HEAD_SIZE) {
        ptr = bytes.long(ptr);
        var nTags = bytes.long(ptr);
        var level = new ZoomLevel(this.file);
        levels.push(level);
        var end = ptr + 8 + 20 * nTags;
        for (ptr += 8; ptr < end; ptr += 20) {
          level.set(bytes.short(ptr), bytes.long(ptr + 4), bytes.long(ptr + 12));
        }
      }
      return levels;
    }
  }], [{
    key: "MAX_HEAD_SIZE",
    value: 8192,
    enumerable: true
  }]);

  return ZIF;
})();

var ZoomLevel = (function () {
  function ZoomLevel(file) {
    _classCallCheck(this, ZoomLevel);

    this.file = file;
    this.meta = new Map();
    this.tilesInfos = null;
  }

  _createClass(ZoomLevel, [{
    key: "getTilesInfos",
    value: function getTilesInfos() {
      if (this.tilesInfos === null) {
        this.tilesInfos = Promise.all([this.getTileOffsets(), this.getTileSizes()]).then(ZoomLevel.transpose);
      }
      return this.tilesInfos;
    }
  }, {
    key: "getTile",
    value: function getTile(x, y) {
      var _this2 = this;

      return this.getTilesInfos().then(function (infos) {
        return _this2.subFile(infos[_this2.xy2num(x, y)]);
      });
    }
  }, {
    key: "set",
    value: function set(k, v1, v2) {
      return this.meta.set(k, [v1, v2]);
    }
  }, {
    key: "get",
    value: function get(k) {
      return this.meta.get(k[0])[k[1]];
    }
  }, {
    key: "dimensions",
    value: function dimensions() {
      return [this.get(ZoomLevel.m_width), this.get(ZoomLevel.m_height)];
    }
  }, {
    key: "widthInTiles",
    value: function widthInTiles() {
      return Math.ceil(this.get(ZoomLevel.m_width) / this.get(ZoomLevel.m_tilesize));
    }
  }, {
    key: "heightInTiles",
    value: function heightInTiles() {
      return Math.ceil(this.get(ZoomLevel.m_height) / this.get(ZoomLevel.m_tilesize));
    }
  }, {
    key: "xy2num",
    value: function xy2num(x, y) {
      return x + y * this.widthInTiles();
    }
  }, {
    key: "subFile",
    value: function subFile(posAndSize) {
      var _posAndSize = _slicedToArray(posAndSize, 2);

      var pos = _posAndSize[0];
      var size = _posAndSize[1];

      return this.file.getBytes(pos, pos + size).then(function (bytes) {
        return new Blob([bytes.u8], { "type": "image/jpeg" });
      });
    }
  }, {
    key: "getTileOffsets",
    value: function getTileOffsets() {
      var count = this.get(ZoomLevel.m_count);
      var tagval = this.get(ZoomLevel.m_pos);
      if (count === 1) return Promise.resolve([tagval]);
      return this.getUintArray(tagval, count, 8);
    }
  }, {
    key: "getTileSizes",
    value: function getTileSizes() {
      var count = this.get(ZoomLevel.m_count);
      var tagval = this.get(ZoomLevel.m_size);
      if (count < 3) {
        return Promise.resolve([tagval | 0, tagval / 0x100000000 | 0].slice(0, count));
      }
      return this.getUintArray(tagval, count, 4);
    }
  }, {
    key: "getUintArray",
    value: function getUintArray(pos, count, bytesPerNum) {
      return this.file.getBytes(pos, pos + bytesPerNum * count).then(function (bytes) {
        var res = new Array(bytes.length / bytesPerNum);
        for (var i = 0; i < res.length; i++) res[i] = bytes.readLittleEndian(i * bytesPerNum, bytesPerNum);
        return res;
      });
    }
  }], [{
    key: "transpose",
    value: function transpose(arrarr) {
      return arrarr[0].map(function (val, i) {
        return arrarr.map(function (arr) {
          return arr[i];
        });
      });
    }
  }, {
    key: "m_width",
    value: [0x0100, 1],
    enumerable: true
  }, {
    key: "m_height",
    value: [0x0101, 1],
    enumerable: true
  }, {
    key: "m_tilesize",
    value: [0x0142, 1],
    enumerable: true
  }, {
    key: "m_count",
    value: [0x0144, 0],
    enumerable: true
  }, {
    key: "m_pos",
    value: [0x0144, 1],
    enumerable: true
  }, {
    key: "m_size",
    value: [0x0145, 1],
    enumerable: true
  }]);

  return ZoomLevel;
})();

var Bytes = (function () {
  // Little endian bytes

  function Bytes(buffer) {
    _classCallCheck(this, Bytes);

    this.u8 = new Uint8Array(buffer);
    this.length = this.u8.length;
  }

  _createClass(Bytes, [{
    key: "readLittleEndian",
    value: function readLittleEndian(pos, bytes) {
      // Warning: javascript cannot store integers larger than 52 bits
      var multiplier = 1,
          res = 0;
      for (var i = pos; i < pos + bytes; i++) {
        res += multiplier * (this.u8[i] | 0);
        multiplier *= 0x100;
      }
      return res;
    }
  }, {
    key: "short",
    value: function short(pos) {
      return this.u8[pos] | this.u8[pos + 1] << 8;
    }
  }, {
    key: "int",
    value: function int(pos) {
      return this.u8[pos] | this.u8[pos + 1] << 8 | this.u8[pos + 2] << 16 | this.u8[pos + 3] << 24;
    }
  }, {
    key: "long",
    value: function long(pos) {
      return this.readLittleEndian(pos, 8);
    }
  }]);

  return Bytes;
})();

var LocalFile = (function () {
  function LocalFile(file) {
    _classCallCheck(this, LocalFile);

    this.file = file;
  }

  _createClass(LocalFile, [{
    key: "getBytes",
    value: function getBytes(begin, end) {
      var _this3 = this;

      return new Promise(function (accept, reject) {
        var r = new FileReader();
        r.onload = function (evt) {
          return accept(new Bytes(r.result));
        };
        r.onerror = function (evt) {
          return reject(r.error);
        };
        r.readAsArrayBuffer(_this3.file.slice(begin, end));
      });
    }
  }]);

  return LocalFile;
})();
