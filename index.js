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
const DEFAULT_CACHE_MAX = 10_000;
const MAX_CACHE_MAX = 0xffffffff;
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

function normalizeCacheCapacity(options = {}) {
  if (options.cache === false) {
    return 0;
  }

  const max = options.cache?.max ?? DEFAULT_CACHE_MAX;
  if (!Number.isSafeInteger(max) || max <= 0 || max > MAX_CACHE_MAX) {
    throw new Error('opts.cache.max should be a positive 32-bit integer');
  }
  return max;
}

function normalizeNetworkOptions(options = {}) {
  return [
    Boolean(options.includeAliasedNetworks),
    Boolean(options.includeNetworksWithoutData),
    Boolean(options.skipEmptyValues),
  ];
}

function normalizeNetworkPageOptions(options = {}) {
  const limit = options.limit ?? 1000;
  const offset = options.offset ?? 0;
  if (!Number.isSafeInteger(limit) || limit <= 0 || limit > MAX_CACHE_MAX) {
    throw new Error('options.limit should be a positive 32-bit integer');
  }
  if (!Number.isSafeInteger(offset) || offset < 0 || offset > MAX_CACHE_MAX) {
    throw new Error('options.offset should be a non-negative 32-bit integer');
  }
  return [...normalizeNetworkOptions(options), limit, offset];
}

function normalizeNetworkPageGeneratorOptions(options = {}) {
  return {
    ...options,
    limit: options.pageSize ?? options.limit ?? 1000,
    offset: options.offset ?? 0,
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

class PathLookup {
  constructor(reader, path) {
    this._reader = reader;
    this._pathId = reader._reader.compilePath(path);
    this.path = Object.freeze([...path]);
  }

  get(ipAddress) {
    return this._reader._reader.getCompiledPath(ipAddress, this._pathId);
  }

  getMany(ipAddresses) {
    return this._reader._reader.getManyCompiledPath(ipAddresses, this._pathId);
  }
}

class Reader {
  constructor(database, options = {}) {
    if (!Buffer.isBuffer(database)) {
      throw new Error(`maxmind-rs expects an instance of Buffer, got: ${typeof database}`);
    }
    this._mode = MODE_BUFFER;
    this._filepath = null;
    this._watchFilepath = null;
    this._watchListener = null;
    this._watchReloadPromise = Promise.resolve();
    this._lastReloadError = null;
    this._cacheCapacity = normalizeCacheCapacity(options);
    this._reader = new native.NativeReader(database, this._cacheCapacity);
    this.metadata = normalizeMetadata(this._reader.metadata());
    this.options = options;
  }

  static open(filepath, options = {}) {
    const mode = normalizeMode(options.mode);
    const reader = Object.create(Reader.prototype);
    reader._mode = mode;
    reader._filepath = filepath;
    reader._watchFilepath = null;
    reader._watchListener = null;
    reader._watchReloadPromise = Promise.resolve();
    reader._lastReloadError = null;
    reader._cacheCapacity = normalizeCacheCapacity(options);
    reader._reader = native.openReader(filepath, mode, reader._cacheCapacity);
    reader.metadata = normalizeMetadata(reader._reader.metadata());
    reader.options = options;
    return reader;
  }

  get closed() {
    return this._reader.closed;
  }

  get lastReloadError() {
    return this._lastReloadError;
  }

  load(database) {
    try {
      this._reader.load(database);
      this.metadata = normalizeMetadata(this._reader.metadata());
      this._lastReloadError = null;
    } catch (error) {
      this._lastReloadError = error;
      throw error;
    }
  }

  reload() {
    if (!this._filepath) {
      throw new Error('Cannot reload a buffer-backed Reader');
    }
    try {
      this._reader.reloadFromFile(this._filepath, this._mode);
      this.metadata = normalizeMetadata(this._reader.metadata());
      this._lastReloadError = null;
    } catch (error) {
      this._lastReloadError = error;
      throw error;
    }
  }

  close() {
    if (this._watchFilepath && this._watchListener) {
      fs.unwatchFile(this._watchFilepath, this._watchListener);
      this._watchFilepath = null;
      this._watchListener = null;
    }
    this._reader.close();
  }

  _queueWatchedReload(filepath, mode, hook) {
    const reload = () => this._reloadWatchedFile(filepath, mode, hook);
    this._watchReloadPromise = this._watchReloadPromise.then(reload, reload);
  }

  async _reloadWatchedFile(filepath, mode, hook) {
    if (!(await waitForFile(filepath))) {
      return;
    }
    if (this.closed || this._watchFilepath !== filepath) {
      return;
    }

    try {
      if (mode === MODE_BUFFER) {
        const database = await readFile(filepath);
        if (this.closed || this._watchFilepath !== filepath) {
          return;
        }
        this.load(database);
      } else {
        this.reload();
      }
      if (!this.closed && this._watchFilepath === filepath && hook) {
        hook();
      }
    } catch (error) {
      this._lastReloadError = error;
    }
  }

  clearCache() {
    this._reader.clearCache();
  }

  cacheStats() {
    return this._reader.cacheStats();
  }

  get(ipAddress) {
    return this._reader.get(ipAddress);
  }

  getPath(ipAddress, path) {
    return this._reader.getPath(ipAddress, path);
  }

  path(path) {
    return new PathLookup(this, path);
  }

  getWithPrefixLength(ipAddress) {
    return this._reader.getWithPrefixLength(ipAddress);
  }

  getMany(ipAddresses) {
    return this._reader.getMany(ipAddresses);
  }

  getManyPath(ipAddresses, path) {
    return this._reader.getManyPath(ipAddresses, path);
  }

  networks(options = {}) {
    return this._reader.networks(null, ...normalizeNetworkOptions(options));
  }

  within(cidr, options = {}) {
    return this._reader.networks(cidr, ...normalizeNetworkOptions(options));
  }

  networksPage(options = {}) {
    return this._reader.networksPage(null, ...normalizeNetworkPageOptions(options));
  }

  withinPage(cidr, options = {}) {
    return this._reader.networksPage(cidr, ...normalizeNetworkPageOptions(options));
  }

  *networkPages(options = {}) {
    let pageOptions = normalizeNetworkPageGeneratorOptions(options);
    while (true) {
      const page = this.networksPage(pageOptions);
      yield page;
      if (page.nextOffset === null) {
        return;
      }
      pageOptions = { ...pageOptions, offset: page.nextOffset };
    }
  }

  *withinPages(cidr, options = {}) {
    let pageOptions = normalizeNetworkPageGeneratorOptions(options);
    while (true) {
      const page = this.withinPage(cidr, pageOptions);
      yield page;
      if (page.nextOffset === null) {
        return;
      }
      pageOptions = { ...pageOptions, offset: page.nextOffset };
    }
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

    const watchListener = () => {
      reader._queueWatchedReload(filepath, mode, options.watchForUpdatesHook);
    };

    fs.watchFile(filepath, watcherOptions, watchListener);
    reader._watchFilepath = filepath;
    reader._watchListener = watchListener;
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
  PathLookup,
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
