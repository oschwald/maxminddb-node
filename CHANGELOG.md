# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Updated the Rust `maxminddb` dependency to 0.30.0, inheriting safer corrupt
  database traversal and faster path and record decoding.
- Invalid UTF-8 in decoded MMDB string values and map keys now follows Node's
  native conversion behavior and is represented with replacement characters.

### Performance

- Decode MMDB strings and map keys directly from raw bytes into JavaScript
  strings, avoiding redundant UTF-8 decoding in Rust and Node.

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
