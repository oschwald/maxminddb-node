# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Added `networksPath()` and `withinPath()` for selectively decoding one value
  while iterating database networks.

### Changed

- Updated the Rust `maxminddb` dependency to 0.30.0, inheriting safer corrupt
  database traversal and faster path and record decoding.
- Invalid UTF-8 in decoded MMDB string values and map keys now follows Node's
  native conversion behavior and is represented with replacement characters.
- `MODE_BUFFER` now uses the same native owned-memory reader as `MODE_MEMORY`;
  the constant remains available as a compatibility alias.
- Network iterators now close their native cursor when a `for...of` loop exits
  early.
- Compiled `PathLookup` instances can be closed explicitly, release their
  native path automatically after collection, and survive watched reader swaps.
- Simplified native reader state to use exclusive mutation directly, removing
  unnecessary runtime borrow checks and their impossible borrow-failure paths.
- Stopped exposing undocumented native reader constructors and helpers from the
  JavaScript entry point, leaving `nativeVersion()` as the supported diagnostic.

### Performance

- Moved gzip input detection into the native readers, eliminating an extra
  asynchronous file open, read, and close during path-based opens.
- Decode MMDB strings and map keys directly from raw bytes into JavaScript
  strings, avoiding redundant UTF-8 decoding in Rust and Node.
- Reduced lookup and batch allocation by reading IP strings directly from V8
  and constructing batch result arrays in place.
- Reuse records with shared data offsets within cached network cursor pages,
  avoiding repeated decoding during large network walks.
- Format network strings on the stack and construct network-result pairs in
  place, avoiding two heap allocations per iterated network.
- Construct decoded arrays and objects in a single pass and carry JavaScript
  decode context through nested values, reducing allocations and TLS access.
- Use a protected LRU segment for caches up to 10,000 records, retaining
  frequently reused records during broad scans without increasing capacity.
- Open owned-memory file modes on a worker thread and read directly into Rust
  memory, avoiding a full intermediate Node `Buffer` and event-loop blocking.
- Store common compiled paths inline, avoiding a heap allocation on each hot
  path lookup.
- Return native network pages directly when no buffered records remain,
  avoiding an extra JavaScript array allocation and copy per page.
- Enable thin link-time optimization for release builds, reducing the native
  binary size and allowing optimization across crate boundaries.

### Development

- Added focused unit coverage for the custom IPv4 parser, IPv4 prefix
  translation, signed path indexes, and inline path storage boundary.
- Removed the unused eager native network collector; the public API continues
  to use the lower-memory native cursor implementation.

## [0.2.1] - 2026-07-03

* Test fix. No other changes.

## [0.2.0] - 2026-07-03

### Changed

- Updated the Rust `maxminddb` dependency to 0.29.0 and refreshed Rust and npm
  development dependencies.
- Coalesced watched file reload events so bursts trigger fewer duplicate reloads
  and hook calls while preserving serialized reloads.

### Performance

- Decoded path lookup results directly into JavaScript values to reduce
  intermediate allocations for `getPath()`, `getManyPath()`, and compiled path
  lookups.
- Decoded network iteration records directly into JavaScript values to reduce
  intermediate allocations for `networks()`, `within()`, and paginated network
  iteration.

## [0.1.0] - 2026-06-14

### Added

- Initial Rust-backed Node.js module for MaxMind DB files, published as
  `@oschwald/maxminddb`.
- Added a `node-maxmind`-compatible API with `open()`, `Reader`, `get()`,
  `getWithPrefixLength()`, `load()`, `metadata`, `validate()`, and legacy
  `init()`/`openSync()` error behavior.
- Added file-backed and buffer-backed readers with `MODE_AUTO`, `MODE_MMAP`,
  `MODE_MEMORY`, and `MODE_BUFFER` open modes.
- Added optional watched reloads with serialized reload handling,
  `lastReloadError`, and explicit watcher cleanup on `reader.close()`.
- Added native LRU caching of materialized records with `cacheStats()` and
  `clearCache()`.
- Added path lookup extensions with `getPath()`, `getManyPath()`, and compiled
  `reader.path()` lookups.
- Added batch lookup support with `getMany()`.
- Added lazy network iteration via native cursors, including `networks()`,
  `within()`, `networkPages()`, `withinPages()`, and `NetworkIterator#nextPage()`.
- Added TypeScript declarations for the public API.
- Added benchmark tooling for comparing throughput against `node-maxmind`.
- Added npm trusted publishing with prebuilt native binaries for Linux x64 GNU,
  Linux arm64 GNU, macOS x64, macOS arm64, Windows x64 MSVC, and Windows arm64
  MSVC.

### Changed

- The npm package is scoped as `@oschwald/maxminddb`; the Rust crate package is named
  `maxminddb-node` to avoid colliding with the upstream Rust `maxminddb` crate.
- Package metadata includes repository, homepage, bugs, author, export map, and
  ISC license metadata.
- Requires Node.js 20 or newer.

### Fixed

- Rejected gzip database inputs before opening.
- Hardened watched reloads so failed reloads keep the existing reader active.
- Hardened streamed large-file reads so truncated or growing files are rejected
  instead of returning partially initialized buffers.
- Added decode corpus regressions for mixed MaxMind DB value types.

### Performance

- Used memory-mapped file reads by default for fast opens and low RSS.
- Added direct N-API decoding paths, cached property descriptor names, and an
  IPv4 parser fast path for hot lookups.
- Added batch lookup and native cursor APIs to reduce JavaScript/native boundary
  crossings.

### Development

- Added CI for Node 20, 22, and 24, plus macOS and Windows coverage.
- Added Rust formatting, `cargo check`, clippy, TypeScript, Node test, npm pack,
  and packed-package smoke-test validation.

[Unreleased]: https://github.com/oschwald/maxminddb-node/compare/v0.2.1...HEAD
[0.2.0]: https://github.com/oschwald/maxminddb-node/releases/tag/v0.2.0
[0.1.0]: https://github.com/oschwald/maxminddb-node/releases/tag/v0.1.0
