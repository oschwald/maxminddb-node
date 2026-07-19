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
    `Unable to load maxminddb native binding. Tried:\n${attempted.join('\n')}`
  );
  error.code = 'ERR_MAXMINDDB_NATIVE_BINDING_NOT_FOUND';
  throw error;
}

const native = loadNativeBinding();
const pathFinalizer = new FinalizationRegistry(({ nativeReader, pathId }) => {
  try {
    nativeReader.releasePath(pathId);
  } catch {
    // Reader shutdown may have already released all compiled paths.
  }
});

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

function normalizeNetworkPageSize(value = 1000) {
  if (!Number.isSafeInteger(value) || value <= 0 || value > MAX_CACHE_MAX) {
    throw new Error('page size should be a positive 32-bit integer');
  }
  return value;
}

function normalizeNetworkIteratorOptions(options = {}) {
  return [
    normalizeNetworkOptions(options),
    normalizeNetworkPageSize(options.pageSize ?? 1000),
  ];
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

class PathLookup {
  constructor(reader, path) {
    this._reader = reader;
    this.path = Object.freeze([...path]);
    this._closed = false;
    this._compileFor(reader._reader);
  }

  get(ipAddress) {
    this._ensureCompiled();
    return this._nativeReader.getCompiledPath(ipAddress, this._pathId);
  }

  getMany(ipAddresses) {
    this._ensureCompiled();
    return this._nativeReader.getManyCompiledPath(ipAddresses, this._pathId);
  }

  close() {
    if (!this._closed) {
      pathFinalizer.unregister(this);
      this._nativeReader.releasePath(this._pathId);
      this._closed = true;
    }
  }

  _ensureCompiled() {
    if (this._closed) {
      throw new Error('Path lookup is closed.');
    }
    if (this._nativeReader !== this._reader._reader) {
      pathFinalizer.unregister(this);
      this._nativeReader.releasePath(this._pathId);
      this._compileFor(this._reader._reader);
    }
  }

  _compileFor(nativeReader) {
    this._nativeReader = nativeReader;
    this._pathId = nativeReader.compilePath(this.path);
    pathFinalizer.register(
      this,
      { nativeReader: this._nativeReader, pathId: this._pathId },
      this
    );
  }
}

class NetworkIterator {
  constructor(reader, cidr, options = {}, path = null) {
    const [networkOptions, pageSize] = normalizeNetworkIteratorOptions(options);
    this._cursor = reader._reader.networkCursor(cidr, ...networkOptions, path);
    this._pageSize = pageSize;
    this._page = [];
    this._index = 0;
    this._done = false;
    this.path = path == null ? null : Object.freeze([...path]);
  }

  [Symbol.iterator]() {
    return this;
  }

  return() {
    this.close();
    return { done: true, value: undefined };
  }

  next() {
    if (this._done) {
      return { done: true, value: undefined };
    }

    if (this._index >= this._page.length) {
      this._page = this._cursor.nextPage(this._pageSize);
      this._index = 0;
      if (this._page.length === 0) {
        this.close();
        return { done: true, value: undefined };
      }
    }

    const value = this._page[this._index];
    this._index += 1;
    return { done: false, value };
  }

  nextPage(pageSize = this._pageSize) {
    pageSize = normalizeNetworkPageSize(pageSize);
    if (this._done) {
      return [];
    }

    const page = [];
    while (page.length < pageSize && this._index < this._page.length) {
      page.push(this._page[this._index]);
      this._index += 1;
    }

    if (page.length < pageSize) {
      const nativePage = this._cursor.nextPage(pageSize - page.length);
      page.push(...nativePage);
      if (nativePage.length === 0) {
        this.close();
      }
    }

    return page;
  }

  *pages(pageSize = this._pageSize) {
    pageSize = normalizeNetworkPageSize(pageSize);
    while (true) {
      const page = this.nextPage(pageSize);
      if (page.length === 0) {
        return;
      }
      yield page;
    }
  }

  close() {
    if (!this._done) {
      this._cursor.close();
      this._done = true;
      this._page = [];
      this._index = 0;
    }
  }
}

class Reader {
  constructor(database, options = {}) {
    if (!Buffer.isBuffer(database)) {
      throw new Error(`maxminddb expects an instance of Buffer, got: ${typeof database}`);
    }
    this._mode = MODE_BUFFER;
    this._filepath = null;
    this._watchFilepath = null;
    this._watchListener = null;
    this._watchReloadActive = false;
    this._watchReloadPending = false;
    this._watchReloadPromise = Promise.resolve();
    this._lastReloadError = null;
    this._cacheCapacity = normalizeCacheCapacity(options);
    this._reader = new native.NativeReader(database, this._cacheCapacity);
    this.metadata = normalizeMetadata(this._reader.metadata());
    this.options = options;
  }

  static open(filepath, options = {}) {
    const mode = normalizeMode(options.mode);
    return Reader._fromNative(
      filepath,
      mode,
      options,
      native.openReader(filepath, mode, normalizeCacheCapacity(options))
    );
  }

  static async openOwned(filepath, mode, options = {}) {
    return Reader._fromNative(
      filepath,
      mode,
      options,
      await native.openReaderAsync(filepath, normalizeCacheCapacity(options))
    );
  }

  static _fromNative(filepath, mode, options, nativeReader) {
    const reader = Object.create(Reader.prototype);
    reader._mode = mode;
    reader._filepath = filepath;
    reader._watchFilepath = null;
    reader._watchListener = null;
    reader._watchReloadActive = false;
    reader._watchReloadPending = false;
    reader._watchReloadPromise = Promise.resolve();
    reader._lastReloadError = null;
    reader._cacheCapacity = normalizeCacheCapacity(options);
    reader._reader = nativeReader;
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
    this._watchReloadPending = false;
    this._reader.close();
  }

  _queueWatchedReload(filepath, mode, hook) {
    this._watchReloadPending = true;
    if (this._watchReloadActive) {
      return;
    }

    this._watchReloadActive = true;
    const drainReloads = async () => {
      try {
        while (this._watchReloadPending) {
          this._watchReloadPending = false;
          await this._reloadWatchedFile(filepath, mode, hook);
        }
      } finally {
        this._watchReloadActive = false;
        if (this._watchReloadPending) {
          this._queueWatchedReload(filepath, mode, hook);
        }
      }
    };

    this._watchReloadPromise = this._watchReloadPromise.then(
      drainReloads,
      drainReloads
    );
  }

  async _reloadWatchedFile(filepath, mode, hook) {
    if (!(await waitForFile(filepath))) {
      return;
    }
    if (this.closed || this._watchFilepath !== filepath) {
      return;
    }

    try {
      if (mode === MODE_MEMORY || mode === MODE_BUFFER) {
        const replacement = await native.openReaderAsync(
          filepath,
          this._cacheCapacity
        );
        if (this.closed || this._watchFilepath !== filepath) {
          replacement.close();
          return;
        }
        const metadata = normalizeMetadata(replacement.metadata());
        const previous = this._reader;
        this._reader = replacement;
        this.metadata = metadata;
        this._lastReloadError = null;
        previous.close();
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
    return new NetworkIterator(this, null, options);
  }

  within(cidr, options = {}) {
    return new NetworkIterator(this, cidr, options);
  }

  networksPath(path, options = {}) {
    return new NetworkIterator(this, null, options, path);
  }

  withinPath(cidr, path, options = {}) {
    return new NetworkIterator(this, cidr, options, path);
  }

  *networkPages(options = {}) {
    yield* this.networks(options).pages(options.pageSize ?? 1000);
  }

  *withinPages(cidr, options = {}) {
    yield* this.within(cidr, options).pages(options.pageSize ?? 1000);
  }
}

async function open(filepath, opts, cb) {
  assert(!cb, legacyErrorMessage);
  const options = opts || {};
  await assertNotGzipFile(filepath);

  const mode = normalizeMode(options.mode);
  const reader =
    mode === MODE_MEMORY || mode === MODE_BUFFER
      ? await Reader.openOwned(filepath, mode, options)
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
  NetworkIterator,
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
