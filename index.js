'use strict';

const assert = require('node:assert');
const fs = require('node:fs');
const net = require('node:net');
const { join } = require('node:path');

const platformTriples = {
  linux: {
    x64: ['linux-x64-gnu', 'linux-x64-musl'],
    arm64: ['linux-arm64-gnu', 'linux-arm64-musl'],
    arm: ['linux-arm-gnueabihf', 'linux-arm-musleabihf'],
    ppc64: ['linux-ppc64-gnu'],
    s390x: ['linux-s390x-gnu'],
    riscv64: ['linux-riscv64-gnu'],
  },
  darwin: {
    x64: ['darwin-x64'],
    arm64: ['darwin-arm64'],
    universal: ['darwin-universal'],
  },
  win32: {
    x64: ['win32-x64-msvc', 'win32-x64-gnu'],
    arm64: ['win32-arm64-msvc'],
    ia32: ['win32-ia32-msvc'],
  },
  freebsd: {
    x64: ['freebsd-x64'],
  },
};

function nativeCandidates() {
  const triples = platformTriples[process.platform]?.[process.arch] ?? [];
  return [
    ...triples.map((triple) => `index.${triple}.node`),
    'index.node',
  ].map((name) => join(__dirname, name));
}

function loadNativeBinding() {
  const attempted = [];
  for (const candidate of nativeCandidates()) {
    attempted.push(candidate);
    if (fs.existsSync(candidate)) {
      return require(candidate);
    }
  }

  const error = new Error(
    `Unable to load maxmind-rs native binding. Tried:\n${attempted.join('\n')}`
  );
  error.code = 'ERR_MAXMIND_RS_NATIVE_BINDING_NOT_FOUND';
  throw error;
}

const native = loadNativeBinding();

const LARGE_FILE_THRESHOLD = 512 * 1024 * 1024;
const STREAM_WATERMARK = 8 * 1024 * 1024;
const legacyErrorMessage = `Maxmind v2 module has changed API.
Please use:
    maxmind.open(dbfile).then(function(lookup) {
        lookup.get(ip);
    });
`;

const MODE_AUTO = 'auto';
const MODE_MMAP = 'mmap';
const MODE_MEMORY = 'memory';
const MODE_BUFFER = 'buffer';

function isGzipBuffer(buffer) {
  return buffer.length >= 2 && buffer[0] === 0x1f && buffer[1] === 0x8b;
}

async function assertNotGzipFile(filepath) {
  const handle = await fs.promises.open(filepath, 'r');
  try {
    const buffer = Buffer.alloc(2);
    const { bytesRead } = await handle.read(buffer, 0, 2, 0);
    if (bytesRead === 2 && isGzipBuffer(buffer)) {
      throw new Error(
        'Looks like you are passing in a file in gzip format, please use mmdb database instead.'
      );
    }
  } finally {
    await handle.close();
  }
}

function normalizeMode(mode) {
  if (mode == null || mode === MODE_AUTO) {
    return MODE_MMAP;
  }
  if (mode === MODE_MMAP || mode === MODE_MEMORY || mode === MODE_BUFFER) {
    return mode;
  }
  throw new Error(`Unsupported open mode: ${mode}`);
}

function normalizeMetadata(metadata) {
  return {
    ...metadata,
    buildEpoch: new Date(Number(metadata.buildEpoch) * 1000),
  };
}

function waitForFile(filepath) {
  for (let i = 0; i < 3; i++) {
    if (fs.existsSync(filepath)) {
      return Promise.resolve(true);
    }
  }

  return new Promise((resolve) => {
    let attempts = 0;
    const retry = () => {
      attempts += 1;
      if (fs.existsSync(filepath)) {
        resolve(true);
      } else if (attempts >= 3) {
        resolve(false);
      } else {
        setTimeout(retry, 500);
      }
    };
    retry();
  });
}

async function readLargeFile(filepath, size) {
  return new Promise((resolve, reject) => {
    const buffer = Buffer.allocUnsafe(size);
    let offset = 0;
    const stream = fs.createReadStream(filepath, {
      highWaterMark: STREAM_WATERMARK,
    });

    stream.on('data', (chunk) => {
      const bufferChunk = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
      bufferChunk.copy(buffer, offset);
      offset += bufferChunk.length;
    });
    stream.on('end', () => {
      stream.close();
      resolve(buffer);
    });
    stream.on('error', reject);
  });
}

async function readFile(filepath) {
  const stat = await fs.promises.stat(filepath);
  return stat.size < LARGE_FILE_THRESHOLD
    ? fs.promises.readFile(filepath)
    : readLargeFile(filepath, stat.size);
}

class Reader {
  constructor(database, options = {}) {
    if (!Buffer.isBuffer(database)) {
      throw new Error(`maxmind-rs expects an instance of Buffer, got: ${typeof database}`);
    }
    this._mode = MODE_BUFFER;
    this._filepath = null;
    this._reader = new native.NativeReader(database);
    this.metadata = normalizeMetadata(this._reader.metadata());
    this.options = options;
  }

  static open(filepath, options = {}) {
    const mode = normalizeMode(options.mode);
    const reader = Object.create(Reader.prototype);
    reader._mode = mode;
    reader._filepath = filepath;
    reader._reader = native.openReader(filepath, mode);
    reader.metadata = normalizeMetadata(reader._reader.metadata());
    reader.options = options;
    return reader;
  }

  get closed() {
    return this._reader.closed;
  }

  load(database) {
    this._reader.load(database);
    this.metadata = normalizeMetadata(this._reader.metadata());
  }

  reload() {
    if (!this._filepath) {
      throw new Error('Cannot reload a buffer-backed Reader');
    }
    this._reader.reloadFromFile(this._filepath, this._mode);
    this.metadata = normalizeMetadata(this._reader.metadata());
  }

  close() {
    this._reader.close();
  }

  get(ipAddress) {
    return this._reader.get(ipAddress);
  }

  getWithPrefixLength(ipAddress) {
    return this._reader.getWithPrefixLength(ipAddress);
  }
}

async function open(filepath, opts, cb) {
  assert(!cb, legacyErrorMessage);
  const options = opts || {};
  await assertNotGzipFile(filepath);

  const mode = normalizeMode(options.mode);
  const reader =
    mode === MODE_BUFFER
      ? new Reader(await readFile(filepath), options)
      : Reader.open(filepath, options);

  if (options.watchForUpdates) {
    if (
      options.watchForUpdatesHook &&
      typeof options.watchForUpdatesHook !== 'function'
    ) {
      throw new Error('opts.watchForUpdatesHook should be a function');
    }

    const watcherOptions = {
      persistent: options.watchForUpdatesNonPersistent !== true,
    };

    fs.watchFile(filepath, watcherOptions, async () => {
      if (!(await waitForFile(filepath))) {
        return;
      }
      if (mode === MODE_BUFFER) {
        reader.load(await readFile(filepath));
      } else {
        reader.reload();
      }
      if (options.watchForUpdatesHook) {
        options.watchForUpdatesHook();
      }
    });
  }

  return reader;
}

function init() {
  throw new Error(legacyErrorMessage);
}

function openSync() {
  throw new Error(legacyErrorMessage);
}

function validate(ipAddress) {
  const version = net.isIP(ipAddress);
  return version === 4 || version === 6;
}

module.exports = {
  ...native,
  Reader,
  init,
  open,
  openSync,
  validate,
  MODE_AUTO,
  MODE_MMAP,
  MODE_MEMORY,
  MODE_BUFFER,
};
