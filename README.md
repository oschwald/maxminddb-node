# maxmind-rs

Rust-backed Node.js reader for MaxMind DB files.

The public API is compatible with the commonly used `maxmind` package from
`node-maxmind` and adds Rust-backed extensions for path lookup, batch lookup,
and network iteration.

## Install

```sh
npm install maxmind-rs
```

## Usage

```js
const maxmind = require('maxmind-rs');

const reader = await maxmind.open('/path/to/GeoIP2-City.mmdb');

console.log(reader.get('8.8.8.8'));
console.log(reader.getWithPrefixLength('8.8.8.8'));
console.log(reader.getPath('8.8.8.8', ['country', 'iso_code']));
```

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

let page = reader.withinPage('81.2.69.0/24', { limit: 100 });
while (page.nextOffset !== null) {
  page = reader.withinPage('81.2.69.0/24', {
    limit: 100,
    offset: page.nextOffset,
  });
}
```

Path elements are strings for map keys and numbers for array indexes. Negative
indexes count from the end of an array.

For high-volume lookup workloads, prefer `getMany()` or `getManyPath()` when
you can batch IPs. They cross the native boundary once for the whole batch and
are significantly faster than calling `get()` in a JavaScript loop.

For large network walks, prefer `networksPage()` or `withinPage()` over
materializing the full `networks()`/`within()` result at once.

## Open Modes

Path-based `open()` defaults to memory-mapped reads:

- `MODE_AUTO`
- `MODE_MMAP`
- `MODE_MEMORY`
- `MODE_BUFFER`

Use `MODE_BUFFER` if you want `open()` to read the file into a Node `Buffer`
before constructing the reader.

```js
const reader = await maxmind.open('/path/to/db.mmdb', {
  mode: maxmind.MODE_MEMORY,
});
```

## Development

```sh
npm install
npm run build
npm test
npm run typecheck
npm run bench -- --compare-node-maxmind
```
