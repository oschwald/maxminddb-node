# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Added `Symbol.dispose` support on `Reader`, `PathLookup`, and
  `NetworkIterator` (requires Node.js 20.4 or newer).

### Changed

- Raised the minimum supported Node.js version in `engines` to `>=20.4`.
- Switched TypeScript declarations to CommonJS `export =` syntax to match the
  runtime `module.exports`; default imports (`import maxmind from
  '@oschwald/maxminddb'`) require `esModuleInterop` when emitting CommonJS, or
  `allowSyntheticDefaultImports` when using a bundler or ESM runtime that
  provides CJS default-import interop.
- Marked the TypeScript constructors of `PathLookup` and `NetworkIterator` as
  `private`; obtain instances through `Reader` methods (`reader.path()`,
  `reader.networks()`, `reader.within()`, etc.).

### Fixed

- Page iterators now close their native network cursor when iteration ends
  early or raises an error, including `NetworkIterator#pages()`,
  `Reader#networkPages()`, and `Reader#withinPages()`.
- `NetworkIterator#next()` and `NetworkIterator#nextPage()` now close their
  native network cursor when reading a page raises an error.
- Corrected `metadata.searchTreeSize` for search trees of 4 GiB or larger,
  preserving the JavaScript Number API without 32-bit overflow.
- Fixed pointer-aliasing UB and unbounded memory growth in the internal
  property-name cache (bounded by both entry count and key length), and
  cleared it on database reloads.
- Prevented `load()` and `reload()` from reopening closed readers, rejected
  closed readers in `load()`, `reload()`, and `reloadAsync()` without
  recording `lastReloadError` (including concurrent closes during reload),
  closed replacement readers if swapping fails before activation, checked
  closed state before IPv4-only validation and async reload I/O, and avoided
  leaking a `napi_ref` if record-cache promotion fails.
- Made `PathLookup` recompilation exception-safe if compiling against a
  reloaded reader fails.
- Validated `watchForUpdatesHook` before opening databases, switched watched
  reload file retries to non-blocking unref'd async checks that retry
  transient `ENOENT`, `EPERM`, and `EBUSY` errors, recorded missing-file and
  access reload errors on `lastReloadError`, and reported synchronous and
  asynchronous `watchForUpdatesHook` failures via `process.emitWarning()`
  instead of recording them as reload failures.
- Improved native binding loading to fall through on failed candidates with a
  warning (`MAXMINDDB_NATIVE_BINDING_FALLBACK`) if a later candidate loads,
  preserve original load errors (`ERR_MAXMINDDB_NATIVE_BINDING_LOAD_FAILED`
  or rethrowing a single load error), and added `darwin-universal` fallback
  entries under `darwin` `x64`/`arm64`.
- Fixed ESM named imports (`import { open, Reader } from '@oschwald/maxminddb'`)
  by exporting shorthand bindings from `index.js`.
- Fixed TypeScript declarations to export `NetworkIterator` on the default
  export, support CJS/ESM interop cleanly via `export =`, allow custom
  non-GeoIP2 database types on `Reader<T>`/`open<T>`, and add generic typing to
  `getPath()`/`getManyPath()`.
- Capped map property descriptor preallocation when decoding untrusted map
  size hints.

### Performance

- Replaced SipHash with `FxBuildHasher` in internal record, property-name, and
  per-page network caches.
- Hoisted decode-scope guards out of batch and network-page loops, fast-pathed
  missing records in `getPaths()`/`getManyPaths()`, and populated network page
  arrays directly.
- Stripped symbols from release `.node` binaries to reduce package size.

### Development

- Enabled Rust unit tests (`cargo test --lib`) using `napi/noop` in
  dev-dependencies and added them to CI and the release script.
- Expanded CI to include ARM64 runners, deduplicated `Mmap`/`Memory` dispatch,
  fixed batch warmup and `--db` validation in `bench/lookup.js`, and cleaned up
  temporary test directories.

## [0.4.0] - 2026-09-12

### Changed

- Upgraded the Rust `maxminddb` dependency from 0.30.0 to 0.32.0, adding
  resource limits for decoding records and metadata, fixing overflowing
  extended data types, and improving record decoding, search-tree traversal,
  and selective path lookups.
- Databases that exceed the new decoding limits now raise the same bad-data
  error as other corrupt databases, including databases that previously
  decoded successfully. These limits apply to lookups, batches, compiled
  paths, and network iteration, as well as metadata during opens and reloads.
- Refreshed Rust and npm development dependencies.

### Performance

- Bind single-record `get()` calls to a validated native reader once, avoiding
  repeated receiver validation and generic borrow tracking in the updated
  N-API bindings. Bound calls retain the reader, use its existing reentrancy
  protection, and follow loads and reloads.

### Development

- Added regression coverage for decoding limits, records at the supported
  limits, empty metadata containers, malformed database fixtures, and
  overflowing extended data types.
- Added coverage ensuring failed resource-limited reloads preserve the
  existing reader and its cached records.

## [0.3.1] - 2026-07-19

### Fixed

- Updated the packed-package smoke test to accept the npm 12
  `npm pack --json` output format, allowing trusted publishing validation to
  complete with the current npm release.

## [0.3.0] - 2026-07-19

### Added

- Added `reloadAsync()` to open and validate replacement databases without
  blocking the JavaScript event loop.
- Added `getPaths()` and `getManyPaths()` for decoding multiple selected values
  with one database lookup per IP.
- Added `networksPath()` and `withinPath()` for selectively decoding one value
  while iterating database networks.

### Changed

- Numeric path components now reject fractional, non-finite, and unsafe
  JavaScript numbers instead of relying on native integer coercion.
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
- Consolidated mutable native reader state behind one runtime borrow, keeping
  reentrant JavaScript access safe without per-field borrow checks.
- Stopped exposing undocumented native reader constructors and helpers from the
  JavaScript entry point, leaving `nativeVersion()` as the supported diagnostic.

### Fixed

- Preserved complete invalid IP values in errors when an overlong string crosses
  the native UTF-8 buffer boundary at a multibyte character.

### Performance

- Moved asynchronous reload work, including memory-mapped reloads triggered by
  file watchers, off the JavaScript event loop.
- Added multi-field projections that reuse one search-tree lookup per IP,
  improving batch throughput when several fields are needed without decoding
  the full record.
- Moved gzip input detection into the native readers, checking owned-file
  prefixes before allocating their buffers and eliminating an extra
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

- Corrected projection benchmark found counts to exclude rows containing only
  missing values.
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

[0.4.0]: https://github.com/oschwald/maxminddb-node/releases/tag/v0.4.0
[0.3.1]: https://github.com/oschwald/maxminddb-node/releases/tag/v0.3.1
[0.3.0]: https://github.com/oschwald/maxminddb-node/releases/tag/v0.3.0
[0.2.1]: https://github.com/oschwald/maxminddb-node/releases/tag/v0.2.1
[0.2.0]: https://github.com/oschwald/maxminddb-node/releases/tag/v0.2.0
[0.1.0]: https://github.com/oschwald/maxminddb-node/releases/tag/v0.1.0
