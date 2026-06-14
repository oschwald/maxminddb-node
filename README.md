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

`cache` is accepted for compatibility. The Rust reader does not currently use
the JavaScript LRU cache.

## Extensions

```js
reader.getPath('8.8.8.8', ['country', 'iso_code']);
reader.getMany(['8.8.8.8', '1.1.1.1']);
reader.getManyPath(['8.8.8.8', '1.1.1.1'], ['country', 'iso_code']);

for (const [network, record] of reader.within('81.2.69.142/31')) {
  console.log(network, record);
}
```

Path elements are strings for map keys and numbers for array indexes. Negative
indexes count from the end of an array.

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
```

