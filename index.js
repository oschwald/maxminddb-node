'use strict';

const assert = require('node:assert');
const fs = require('node:fs');
const net = require('node:net');
const { join } = require('node:path');
const util = require('node:util');

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
    x64: ['darwin-x64', 'darwin-universal'],
    arm64: ['darwin-arm64', 'darwin-universal'],
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

function isMuslLinux() {
  try {
    return fs.readFileSync('/usr/bin/ldd', 'utf8').includes('musl');
  } catch {
    // Some images have no ldd; fall back to the process report.
  }
  const processReport = process.report;
  if (typeof processReport?.getReport !== 'function') {
    return false;
  }
  // Network details are not needed and can make getReport() slow.
  const excludeNetwork = processReport.excludeNetwork;
  try {
    processReport.excludeNetwork = true;
    const sharedObjects = processReport.getReport()?.sharedObjects;
    return (
      Array.isArray(sharedObjects) &&
      sharedObjects.some(
        (file) =>
          typeof file === 'string' &&
          (file.includes('libc.musl-') || file.includes('ld-musl-'))
      )
    );
  } catch {
    return false;
  } finally {
    try {
      processReport.excludeNetwork = excludeNetwork;
    } catch {
      // Ignore runtimes where the setting cannot be restored.
    }
  }
}

function nativeCandidates() {
  let triples = platformTriples[process.platform]?.[process.arch] ?? [];
  if (process.platform === 'linux' && isMuslLinux()) {
    // Try the musl build first so that musl systems such as Alpine do not
    // load the glibc build, or warn after it fails to load.
    const isMuslTriple = (triple) => triple.includes('musl');
    triples = [
      ...triples.filter(isMuslTriple),
      ...triples.filter((triple) => !isMuslTriple(triple)),
    ];
  }
  return [
    ...triples.map((triple) => `index.${triple}.node`),
    'index.node',
  ].map((name) => join(__dirname, name));
}

function loadNativeBinding() {
  const attempted = [];
  const loadErrors = [];
  for (const candidate of nativeCandidates()) {
    attempted.push(candidate);
    if (!fs.existsSync(candidate)) {
      continue;
    }
    try {
      const binding = require(candidate);
      if (loadErrors.length > 0) {
        const details = loadErrors
          .map(({ candidate: failed, err }) => `${failed}: ${err.message}`)
          .join('\n');
        process.emitWarning(
          `Loaded fallback maxminddb native binding ${candidate} after earlier candidate(s) failed:\n${details}`,
          { code: 'MAXMINDDB_NATIVE_BINDING_FALLBACK' }
        );
      }
      return binding;
    } catch (err) {
      loadErrors.push({ candidate, err });
    }
  }

  if (loadErrors.length === 1) {
    throw loadErrors[0].err;
  }

  if (loadErrors.length > 1) {
    const details = loadErrors
      .map(({ candidate, err }) => `${candidate}: ${err.message}`)
      .join('\n');
    const error = new Error(
      `Unable to load maxminddb native binding. Load errors:\n${details}`,
      { cause: loadErrors[0].err }
    );
    error.code = 'ERR_MAXMINDDB_NATIVE_BINDING_LOAD_FAILED';
    throw error;
  }

  const error = new Error(
    `Unable to load maxminddb native binding. Tried:\n${attempted.join('\n')}`
  );
  error.code = 'ERR_MAXMINDDB_NATIVE_BINDING_NOT_FOUND';
  throw error;
}

const native = loadNativeBinding();
const nativeVersion = native.nativeVersion;
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

async function waitForFile(filepath, shouldStop) {
  for (let attempt = 0; attempt < 3; attempt += 1) {
    if (shouldStop()) {
      return false;
    }
    try {
      await fs.promises.access(filepath);
      return true;
    } catch (error) {
      const code = error?.code;
      const isRetryable =
        code === 'ENOENT' || code === 'EPERM' || code === 'EBUSY';
      if (!isRetryable) {
        throw error;
      }
      if (attempt === 2) {
        if (code === 'ENOENT') {
          return false;
        }
        throw error;
      }
    }
    await new Promise((resolve) => {
      const timer = setTimeout(resolve, 500);
      if (typeof timer.unref === 'function') {
        timer.unref();
      }
    });
  }
  return false;
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

  [Symbol.dispose]() {
    this.close();
  }

  _ensureCompiled() {
    if (this._closed) {
      throw new Error('Path lookup is closed.');
    }
    if (this._nativeReader !== this._reader._reader) {
      const previousReader = this._nativeReader;
      const previousPathId = this._pathId;
      const nextReader = this._reader._reader;
      const nextPathId = nextReader.compilePath(this.path);
      pathFinalizer.unregister(this);
      previousReader.releasePath(previousPathId);
      this._nativeReader = nextReader;
      this._pathId = nextPathId;
      pathFinalizer.register(
        this,
        { nativeReader: this._nativeReader, pathId: this._pathId },
        this
      );
    }
  }

  _compileFor(nativeReader) {
    const pathId = nativeReader.compilePath(this.path);
    this._nativeReader = nativeReader;
    this._pathId = pathId;
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
      this._page = this._readNativePage(this._pageSize);
      this._index = 0;
      if (this._done) {
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

    if (this._index >= this._page.length) {
      this._page = [];
      this._index = 0;
      return this._readNativePage(pageSize);
    }

    const page = [];
    while (page.length < pageSize && this._index < this._page.length) {
      page.push(this._page[this._index]);
      this._index += 1;
    }

    if (page.length < pageSize) {
      const nativePage = this._readNativePage(pageSize - page.length);
      for (const record of nativePage) {
        page.push(record);
      }
    }

    return page;
  }

  *pages(pageSize = this._pageSize) {
    pageSize = normalizeNetworkPageSize(pageSize);
    try {
      while (true) {
        const page = this.nextPage(pageSize);
        if (page.length === 0) {
          return;
        }
        yield page;
      }
    } finally {
      this.close();
    }
  }

  _readNativePage(pageSize) {
    try {
      const page = this._cursor.nextPage(pageSize);
      if (page.length === 0) {
        this.close();
      }
      return page;
    } catch (error) {
      this.close();
      throw error;
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

  [Symbol.dispose]() {
    this.close();
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
    this._reloadOutcomeSequence = 0;
    this._cacheCapacity = normalizeCacheCapacity(options);
    this._reader = new native.NativeReader(database, this._cacheCapacity);
    this._get = this._reader.bindGet();
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
    let nativeReader;
    try {
      nativeReader = await native.openReaderAsync(
        filepath,
        mode,
        normalizeCacheCapacity(options)
      );
    } catch (error) {
      takeReadSequence(error);
      throw error;
    }
    return Reader._fromNative(filepath, mode, options, nativeReader);
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
    reader._reloadOutcomeSequence = 0;
    reader._cacheCapacity = normalizeCacheCapacity(options);
    reader._reader = nativeReader;
    reader._get = nativeReader.bindGet();
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
    if (this.closed) {
      throw new Error('Cannot load into a closed Reader');
    }
    const startSequence = native.currentReadSequence();
    try {
      this._reader.load(database);
      this.metadata = normalizeMetadata(this._reader.metadata());
      this._recordReloadSuccess(this._reader.readSequence);
    } catch (error) {
      this._recordReloadFailure(startSequence + 0.5, error);
      throw error;
    }
  }

  reload() {
    if (!this._filepath) {
      throw new Error('Cannot reload a buffer-backed Reader');
    }
    if (this.closed) {
      throw new Error('Cannot reload a closed Reader');
    }
    const startSequence = native.currentReadSequence();
    try {
      this._reader.reloadFromFile(this._filepath, this._mode);
      this.metadata = normalizeMetadata(this._reader.metadata());
      this._recordReloadSuccess(this._reader.readSequence);
    } catch (error) {
      this._recordReloadFailure(startSequence + 0.5, error);
      throw error;
    }
  }

  // Resolves to true when the replacement was installed. If the installed
  // database was read after this call read its file (by load(), reload(),
  // another reloadAsync(), or a watched reload), the replacement is closed
  // and the promise resolves to false without changing the reader.
  async reloadAsync() {
    if (!this._filepath) {
      throw new Error('Cannot reload a buffer-backed Reader');
    }
    if (this.closed) {
      throw new Error('Cannot reload a closed Reader');
    }
    const startSequence = native.currentReadSequence();
    let replacement;
    try {
      replacement = await this._openReplacementAsync(
        this._filepath,
        this._mode
      );
    } catch (error) {
      if (this.closed) {
        throw new Error('Cannot reload a closed Reader');
      }
      this._recordReloadFailure(startSequence + 0.5, error);
      throw error;
    }
    if (this.closed) {
      replacement.close();
      throw new Error('Cannot reload a closed Reader');
    }
    if (this._isSuperseded(replacement)) {
      replacement.close();
      return false;
    }
    try {
      this._replaceNativeReader(replacement);
    } catch (error) {
      if (this._reader !== replacement) {
        replacement.close();
      }
      if (this.closed) {
        throw new Error('Cannot reload a closed Reader');
      }
      this._recordReloadFailure(replacement.readSequence, error);
      throw error;
    }
    return true;
  }

  // Reloads are ordered by the native read sequence, a process-wide counter
  // taken when each database file is opened or buffer loaded.
  //
  // _isSuperseded() is true when the installed database was read after the
  // replacement, so installing it would roll the reader back.
  _isSuperseded(replacement) {
    return this._reader.readSequence > replacement.readSequence;
  }

  // lastReloadError reflects whichever outcome is newest in read order. A
  // success counts from when its database was read. A failure counts from
  // its failed read when the native layer reports one (error.readSequence),
  // from the replacement's read when installing a replacement fails, and
  // otherwise from just after the sequence current when its attempt began,
  // so it supersedes every read that began before the attempt.
  _recordReloadSuccess(readSequence) {
    if (readSequence > this._reloadOutcomeSequence) {
      this._reloadOutcomeSequence = readSequence;
      this._lastReloadError = null;
    }
  }

  _recordReloadFailure(fallbackSequence, error) {
    const sequence = takeReadSequence(error) ?? fallbackSequence;
    if (sequence >= this._reloadOutcomeSequence) {
      this._reloadOutcomeSequence = sequence;
      this._lastReloadError = error;
    }
  }

  _openReplacementAsync(filepath, mode) {
    return native.openReaderAsync(filepath, mode, this._cacheCapacity);
  }

  _replaceNativeReader(replacement) {
    const metadata = normalizeMetadata(replacement.metadata());
    const get = replacement.bindGet();
    const previous = this._reader;
    this._reader = replacement;
    this._get = get;
    this.metadata = metadata;
    previous.close();
    this._recordReloadSuccess(replacement.readSequence);
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

  [Symbol.dispose]() {
    this.close();
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
    const isStale = () => this.closed || this._watchFilepath !== filepath;

    let reloaded = false;
    let replacement;
    let startSequence = native.currentReadSequence();
    try {
      if (!(await waitForFile(filepath, isStale))) {
        if (!isStale()) {
          this._recordReloadFailure(
            startSequence + 0.5,
            new Error(`Database file not found: ${filepath}`)
          );
        }
        return;
      }
      if (isStale()) {
        return;
      }

      startSequence = native.currentReadSequence();
      replacement = await this._openReplacementAsync(filepath, mode);
      if (isStale() || this._isSuperseded(replacement)) {
        replacement.close();
        return;
      }
      this._replaceNativeReader(replacement);
      replacement = null;
      reloaded = true;
    } catch (error) {
      if (replacement && this._reader !== replacement) {
        replacement.close();
      }
      if (!isStale()) {
        this._recordReloadFailure(
          replacement ? replacement.readSequence : startSequence + 0.5,
          error
        );
      }
    }

    if (reloaded && !isStale() && hook) {
      try {
        const result = hook();
        if (result && typeof result.then === 'function') {
          Promise.resolve(result).catch(emitHookWarning);
        }
      } catch (error) {
        emitHookWarning(error);
      }
    }
  }

  clearCache() {
    this._reader.clearCache();
  }

  cacheStats() {
    return this._reader.cacheStats();
  }

  get(ipAddress) {
    return this._get(ipAddress);
  }

  getPath(ipAddress, path) {
    return this._reader.getPath(ipAddress, path);
  }

  getPaths(ipAddress, paths) {
    return this._reader.getPaths(ipAddress, paths);
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

  getManyPaths(ipAddresses, paths) {
    return this._reader.getManyPaths(ipAddresses, paths);
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
    yield* this.networks(options).pages();
  }

  *withinPages(cidr, options = {}) {
    yield* this.within(cidr, options).pages();
  }
}

async function open(filepath, opts, cb) {
  assert(!cb, legacyErrorMessage);
  const options = opts || {};

  const mode = normalizeMode(options.mode);
  if (
    options.watchForUpdates &&
    options.watchForUpdatesHook &&
    typeof options.watchForUpdatesHook !== 'function'
  ) {
    throw new Error('opts.watchForUpdatesHook should be a function');
  }

  const reader =
    mode === MODE_MEMORY || mode === MODE_BUFFER
      ? await Reader.openOwned(filepath, mode, options)
      : Reader.open(filepath, options);

  if (options.watchForUpdates) {
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

// Native read failures carry the failed read's sequence as readSequence.
// Returns it and removes the property so it never reaches callers.
function takeReadSequence(error) {
  if (error === null || typeof error !== 'object') {
    return undefined;
  }
  const sequence = error.readSequence;
  delete error.readSequence;
  return typeof sequence === 'number' ? sequence : undefined;
}

function emitHookWarning(error) {
  if (error instanceof Error || typeof error === 'string') {
    process.emitWarning(error);
  } else {
    process.emitWarning(new Error(util.inspect(error), { cause: error }));
  }
}

function validate(ipAddress) {
  const version = net.isIP(ipAddress);
  if (version === 4) {
    return true;
  }
  // net.isIP() accepts IPv6 zone identifiers such as fe80::1%eth0, but
  // get() and the other lookup methods reject them. Keep validate() in
  // agreement with the lookups so a validated address never throws.
  return version === 6 && !ipAddress.includes('%');
}

module.exports = {
  nativeVersion,
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
