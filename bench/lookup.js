#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');

const maxmind = require('..');

const DEFAULT_DBS = [
  '/var/lib/GeoIP/GeoIP2-City.mmdb',
  '/var/lib/GeoIP/GeoLite2-City.mmdb',
].filter((db) => fs.existsSync(db));

const DEFAULT_COUNT = 200_000;
const DEFAULT_WARMUP = 50_000;

function parseArgs(argv) {
  const options = {
    count: DEFAULT_COUNT,
    warmup: DEFAULT_WARMUP,
    compareNodeMaxmind: false,
    dbs: [],
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--compare-node-maxmind') {
      options.compareNodeMaxmind = true;
    } else if (arg === '--count') {
      options.count = parsePositiveInteger(argv[++i], '--count');
    } else if (arg === '--warmup') {
      options.warmup = parsePositiveInteger(argv[++i], '--warmup');
    } else if (arg === '--db') {
      options.dbs.push(argv[++i]);
    } else if (arg === '--help' || arg === '-h') {
      printHelp();
      process.exit(0);
    } else {
      options.dbs.push(arg);
    }
  }

  if (options.dbs.length === 0) {
    options.dbs = DEFAULT_DBS;
  }
  if (options.dbs.length === 0) {
    throw new Error('No database paths provided and no default /var/lib/GeoIP databases found');
  }

  return options;
}

function parsePositiveInteger(value, name) {
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number <= 0) {
    throw new Error(`${name} must be a positive integer`);
  }
  return number;
}

function printHelp() {
  console.log(`Usage: node bench/lookup.js [options] [db.mmdb...]

Options:
  --db <path>                Add a database path
  --count <n>                Number of lookup IPs, default ${DEFAULT_COUNT}
  --warmup <n>               Warmup lookup count, default ${DEFAULT_WARMUP}
  --compare-node-maxmind     Compare against ../node-maxmind when available
  -h, --help                 Show this help
`);
}

function makeIps(count) {
  let state = 0x12345678 >>> 0;
  const ips = new Array(count);
  for (let i = 0; i < count; i += 1) {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    ips[i] = `${(state >>> 24) & 255}.${(state >>> 16) & 255}.${(state >>> 8) & 255}.${state & 255}`;
  }
  return ips;
}

function formatRate(count, seconds) {
  return Math.round(count / seconds).toLocaleString('en-US');
}

function rssMb() {
  return process.memoryUsage().rss / 1024 / 1024;
}

function gc() {
  if (global.gc) {
    global.gc();
  }
}

async function benchOpen(label, openFn) {
  gc();
  const rssBefore = rssMb();
  const start = process.hrtime.bigint();
  const reader = await openFn();
  const elapsedMs = Number(process.hrtime.bigint() - start) / 1e6;
  const rssDelta = rssMb() - rssBefore;
  console.log(`${label.padEnd(36)} open ${elapsedMs.toFixed(2).padStart(8)} ms rss ${rssDelta.toFixed(1).padStart(7)} MB`);
  return reader;
}

function warmup(reader, ips, warmupCount, lookup) {
  const count = Math.min(warmupCount, ips.length);
  for (let i = 0; i < count; i += 1) {
    lookup(reader, ips[i]);
  }
}

function benchLookup(label, reader, ips, warmupCount, lookup) {
  warmup(reader, ips, warmupCount, lookup);
  gc();

  let found = 0;
  const start = process.hrtime.bigint();
  for (const ip of ips) {
    if (lookup(reader, ip)) {
      found += 1;
    }
  }
  const elapsed = Number(process.hrtime.bigint() - start) / 1e9;
  console.log(`${label.padEnd(36)} ${formatRate(ips.length, elapsed).padStart(12)}/s found ${found.toLocaleString('en-US').padStart(10)} time ${elapsed.toFixed(3).padStart(7)} s`);
}

function benchMany(label, reader, ips, warmupCount, lookupMany) {
  warmup(reader, ips, warmupCount, (r, ip) => r.get(ip));
  gc();

  const start = process.hrtime.bigint();
  const values = lookupMany(reader, ips);
  let found = 0;
  for (const value of values) {
    if (value) {
      found += 1;
    }
  }
  const elapsed = Number(process.hrtime.bigint() - start) / 1e9;
  console.log(`${label.padEnd(36)} ${formatRate(ips.length, elapsed).padStart(12)}/s found ${found.toLocaleString('en-US').padStart(10)} time ${elapsed.toFixed(3).padStart(7)} s`);
}

async function closeMaybe(reader) {
  if (reader && typeof reader.close === 'function') {
    reader.close();
  }
}

function loadNodeMaxmind() {
  const candidate = path.join(__dirname, '..', '..', 'node-maxmind', 'lib');
  if (!fs.existsSync(candidate)) {
    return null;
  }
  return require(candidate);
}

async function benchDatabase(db, options, ips, nodeMaxmind) {
  console.log(`\n${db}`);

  const uncached = await benchOpen('maxmind-rs cache:false', () =>
    maxmind.open(db, { cache: false })
  );
  benchLookup('get cache:false', uncached, ips, options.warmup, (reader, ip) =>
    reader.get(ip)
  );
  await closeMaybe(uncached);

  const cached = await benchOpen('maxmind-rs default cache', () => maxmind.open(db));
  benchLookup('get default cache', cached, ips, options.warmup, (reader, ip) =>
    reader.get(ip)
  );
  benchLookup('getPath country.iso', cached, ips, options.warmup, (reader, ip) =>
    reader.getPath(ip, ['country', 'iso_code'])
  );
  const countryIso = cached.path(['country', 'iso_code']);
  benchLookup('path country.iso', countryIso, ips, options.warmup, (lookup, ip) =>
    lookup.get(ip)
  );
  benchMany('getMany default cache', cached, ips, options.warmup, (reader, values) =>
    reader.getMany(values)
  );
  await closeMaybe(cached);

  const largerCache = await benchOpen('maxmind-rs cache:100k', () =>
    maxmind.open(db, { cache: { max: 100_000 } })
  );
  benchLookup('get cache:100k', largerCache, ips, options.warmup, (reader, ip) =>
    reader.get(ip)
  );
  benchMany('getMany cache:100k', largerCache, ips, options.warmup, (reader, values) =>
    reader.getMany(values)
  );
  await closeMaybe(largerCache);

  if (nodeMaxmind) {
    const nodeDefault = await benchOpen('node-maxmind default cache', () =>
      nodeMaxmind.open(db)
    );
    benchLookup('node-maxmind get default cache', nodeDefault, ips, options.warmup, (reader, ip) =>
      reader.get(ip)
    );

    const nodeLargerCache = await benchOpen('node-maxmind cache:100k', () =>
      nodeMaxmind.open(db, { cache: { max: 100_000 } })
    );
    benchLookup('node-maxmind get cache:100k', nodeLargerCache, ips, options.warmup, (reader, ip) =>
      reader.get(ip)
    );
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const nodeMaxmind = options.compareNodeMaxmind ? loadNodeMaxmind() : null;
  const ips = makeIps(options.count);

  if (options.compareNodeMaxmind && !nodeMaxmind) {
    console.warn('Skipping node-maxmind comparison; ../node-maxmind/lib was not found');
  }

  for (const db of options.dbs) {
    await benchDatabase(db, options, ips, nodeMaxmind);
  }
}

main().catch((error) => {
  console.error(error.stack || error.message);
  process.exitCode = 1;
});
