# @oschwald/maxminddb

Rust-backed Node.js reader for MaxMind DB files.

The public API is compatible with the commonly used `maxmind` package from
`node-maxmind` and adds Rust-backed extensions for path lookup, batch lookup,
and network iteration.

## Install

Node.js 20 or newer is required.

```sh
npm install @oschwald/maxminddb
```

## Usage

These examples use ES modules. Save them as `.mjs`, or set `"type": "module"`
in your `package.json`.

```js
import maxmind from '@oschwald/maxminddb';

const reader = await maxmind.open('/path/to/GeoIP2-City.mmdb');

console.log(reader.get('8.8.8.8'));
console.log(reader.getWithPrefixLength('8.8.8.8'));
console.log(reader.getPath('8.8.8.8', ['country', 'iso_code']));
```

CommonJS `require()` is also supported for compatibility with existing
`node-maxmind` users.

## Compatibility API

- `open(filepath, options?)`
- `new Reader(buffer, options?)`
- `reader.get(ipAddress)`
- `reader.getWithPrefixLength(ipAddress)`
- `reader.load(buffer)`
- `reader.metadata`
- `validate(ipAddress)`
- `init()` and `openSync()` keep the legacy `node-maxmind` error behavior.

`open()` accepts the existing `node-maxmind` options:

- `cache`
- `watchForUpdates`
- `watchForUpdatesNonPersistent`
- `watchForUpdatesHook`

`cache` controls a native LRU cache of materialized records keyed by MaxMind DB
data offset. The default is 10,000 records, matching `node-maxmind`. Pass
`cache: { max: 1000 }` to tune the cache size or `cache: false` to disable it.
Use `reader.cacheStats()` to inspect hit/miss counters and `reader.clearCache()`
to release cached record references.

The record cache stores JavaScript objects, not compressed database bytes. Large
cache sizes can therefore retain significant heap memory when the database
records are large or lookups touch many distinct data offsets. Cached records
are returned by reference, so mutating a cached record can affect later lookups
for the same data offset until that entry is evicted, `reader.clearCache()` is
called, or the reader is closed. `getPath()`, `getManyPath()`, and compiled
`reader.path()` lookups decode only the requested path and do not populate the
full-record cache.

When `watchForUpdates` is enabled, file-change reloads run serially. A failed
watched reload leaves the existing reader active, stores the failure on
`reader.lastReloadError`, and skips `watchForUpdatesHook`. The next successful
reload clears `lastReloadError` and calls the hook. Close watched readers with
`reader.close()` to remove the file watcher.

## Extensions

```js
reader.getPath('8.8.8.8', ['country', 'iso_code']);
reader.getMany(['8.8.8.8', '1.1.1.1']);
reader.getManyPath(['8.8.8.8', '1.1.1.1'], ['country', 'iso_code']);

const countryCode = reader.path(['country', 'iso_code']);
countryCode.get('8.8.8.8');
countryCode.getMany(['8.8.8.8', '1.1.1.1']);

for (const [network, record] of reader.within('81.2.69.142/31')) {
  console.log(network, record);
}

for (const page of reader.withinPages('81.2.69.0/24', { pageSize: 100 })) {
  for (const [network, record] of page) {
    console.log(network, record);
  }
}

for (const [network, country] of reader.withinPath(
  '81.2.69.0/24',
  ['country', 'iso_code']
)) {
  console.log(network, country);
}
```

Path elements are strings for map keys and numbers for array indexes. Negative
indexes count from the end of an array.

Create compiled path lookups once and reuse them in hot paths. `reader.path()`
parses and stores the path, and the returned `PathLookup` avoids reparsing the
path array on each lookup.

For high-volume lookup workloads, prefer `getMany()` or `getManyPath()` when
you can batch IPs. They cross the native boundary once for the whole batch and
are significantly faster than calling `get()` in a JavaScript loop.

`networks()` and `within()` return lazy iterators backed by native cursors.
Use `networksPath()` or `withinPath()` to decode only a selected field while
walking networks.
For large network walks, use `networkPages()`, `withinPages()`, or
`NetworkIterator#nextPage()` to cross the native boundary once per page rather
than once per network.

## Open Modes

Path-based `open()` defaults to memory-mapped reads:

- `MODE_AUTO`
- `MODE_MMAP`
- `MODE_MEMORY`
- `MODE_BUFFER`

`MODE_MEMORY` and `MODE_BUFFER` asynchronously read the file once into
Rust-owned memory. `MODE_BUFFER` is retained as a compatibility alias.

```js
import maxmind from '@oschwald/maxminddb';

const reader = await maxmind.open('/path/to/db.mmdb', {
  mode: maxmind.MODE_MEMORY,
});
```

Mode tradeoffs:

- `MODE_MMAP`/`MODE_AUTO` opens quickly and keeps RSS low by mapping the
  database file. Replace database files atomically when using watched reloads.
- `MODE_MEMORY`/`MODE_BUFFER` asynchronously read the database into Rust-owned
  memory. They cost more memory at open time but are independent of the source
  file after open.

## Performance Notes

Performance depends on database size, record shape, cache hit rate, CPU, Node
version, and whether the database is warm in the OS page cache. On one local
run with 200,000 generated IPv4 lookups against `/var/lib/GeoIP`, this module
had much faster open times and lower RSS than `node-maxmind`, while cached
single-record lookup throughput was still lower:

| Database | maxminddb default cache | node-maxmind default cache | maxminddb cache:100k | node-maxmind cache:100k |
| --- | ---: | ---: | ---: | ---: |
| GeoIP2-City | 370k/s | 441k/s | 632k/s | 869k/s |
| GeoLite2-City | 452k/s | 498k/s | 715k/s | 1.07M/s |

The same run opened mapped readers in sub-millisecond to low single-digit
milliseconds after warmup, while `node-maxmind` open times were tens of
milliseconds and retained tens to hundreds of MB of RSS. Batch lookups were
faster than JavaScript loops over `get()`, reaching roughly 3.0-3.4M IPs/s in
that run.

Run local benchmarks with:

```sh
npm run bench -- --compare-node-maxmind --db /path/to/db.mmdb
```

## Supported Platforms

The npm package is set up to ship prebuilt native modules for Linux x64 GNU,
Linux arm64 GNU, macOS x64, macOS arm64, Windows x64 MSVC, and Windows arm64
MSVC. The loader also knows platform-specific filenames for additional Linux,
Windows, and FreeBSD targets, but those artifacts are not part of the default
trusted publishing workflow yet. See [RELEASE.md](./RELEASE.md) for the native
artifact strategy.

## Development

```sh
npm install
npm run build
npm test
npm run typecheck
npm run bench -- --compare-node-maxmind
npm run release
npm run bench -- --save-baseline /tmp/maxminddb-baseline.json
npm run bench -- --baseline /tmp/maxminddb-baseline.json --min-ratio 0.9
npm run --silent bench -- --json > bench-results.json
```

See [RELEASE.md](./RELEASE.md) for packaging expectations and the native
prebuild release strategy.

## License

ISC License. See [LICENSE](./LICENSE) for details.
