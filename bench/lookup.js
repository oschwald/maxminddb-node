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
    baseline: null,
    dbs: [],
    json: false,
    minRatio: 0.9,
    saveBaseline: null,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--compare-node-maxmind') {
      options.compareNodeMaxmind = true;
    } else if (arg === '--baseline') {
      options.baseline = argv[++i];
    } else if (arg === '--count') {
      options.count = parsePositiveInteger(argv[++i], '--count');
    } else if (arg === '--json') {
      options.json = true;
    } else if (arg === '--min-ratio') {
      options.minRatio = parsePositiveNumber(argv[++i], '--min-ratio');
    } else if (arg === '--save-baseline') {
      options.saveBaseline = argv[++i];
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

  if (options.baseline == null && options.minRatio !== 0.9) {
    throw new Error('--min-ratio requires --baseline');
  }
  if (options.baseline === undefined || options.saveBaseline === undefined) {
    throw new Error('--baseline and --save-baseline require a path');
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

function parsePositiveNumber(value, name) {
  const number = Number(value);
  if (!Number.isFinite(number) || number <= 0) {
    throw new Error(`${name} must be a positive number`);
  }
  return number;
}

function printHelp() {
  console.log(`Usage: node bench/lookup.js [options] [db.mmdb...]

Options:
  --baseline <path>          Compare rates against a saved JSON baseline
  --db <path>                Add a database path
  --count <n>                Number of lookup IPs, default ${DEFAULT_COUNT}
  --json                     Print machine-readable JSON instead of tables
  --min-ratio <n>            Minimum current/baseline rate, default 0.9
  --save-baseline <path>     Write benchmark results as a JSON baseline
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

function logHuman(options, message) {
  if (!options.json) {
    console.log(message);
  }
}

async function benchOpen(label, openFn, dbResult, options) {
  gc();
  const rssBefore = rssMb();
  const start = process.hrtime.bigint();
  const reader = await openFn();
  const elapsedMs = Number(process.hrtime.bigint() - start) / 1e6;
  const rssDelta = rssMb() - rssBefore;
  dbResult.opens.push({ label, elapsedMs, rssMb: rssDelta });
  logHuman(
    options,
    `${label.padEnd(36)} open ${elapsedMs.toFixed(2).padStart(8)} ms rss ${rssDelta.toFixed(1).padStart(7)} MB`
  );
  return reader;
}

function warmup(reader, ips, warmupCount, lookup) {
  const count = Math.min(warmupCount, ips.length);
  for (let i = 0; i < count; i += 1) {
    lookup(reader, ips[i]);
  }
}

function benchLookup(label, reader, ips, warmupCount, lookup, dbResult, options) {
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
  const result = {
    label,
    count: ips.length,
    found,
    seconds: elapsed,
    rate: ips.length / elapsed,
  };
  dbResult.benchmarks.push(result);
  logHuman(
    options,
    `${label.padEnd(36)} ${formatRate(ips.length, elapsed).padStart(12)}/s found ${found.toLocaleString('en-US').padStart(10)} time ${elapsed.toFixed(3).padStart(7)} s`
  );
  return result;
}

function benchMany(label, reader, ips, warmupCount, lookupMany, dbResult, options) {
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
  const result = {
    label,
    count: ips.length,
    found,
    seconds: elapsed,
    rate: ips.length / elapsed,
  };
  dbResult.benchmarks.push(result);
  logHuman(
    options,
    `${label.padEnd(36)} ${formatRate(ips.length, elapsed).padStart(12)}/s found ${found.toLocaleString('en-US').padStart(10)} time ${elapsed.toFixed(3).padStart(7)} s`
  );
  return result;
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
  logHuman(options, `\n${db}`);
  const dbResult = {
    path: db,
    opens: [],
    benchmarks: [],
  };

  const uncached = await benchOpen(
    'maxmind-rs cache:false',
    () => maxmind.open(db, { cache: false }),
    dbResult,
    options
  );
  benchLookup(
    'get cache:false',
    uncached,
    ips,
    options.warmup,
    (reader, ip) => reader.get(ip),
    dbResult,
    options
  );
  await closeMaybe(uncached);

  const cached = await benchOpen(
    'maxmind-rs default cache',
    () => maxmind.open(db),
    dbResult,
    options
  );
  benchLookup(
    'get default cache',
    cached,
    ips,
    options.warmup,
    (reader, ip) => reader.get(ip),
    dbResult,
    options
  );
  benchLookup(
    'getPath country.iso',
    cached,
    ips,
    options.warmup,
    (reader, ip) => reader.getPath(ip, ['country', 'iso_code']),
    dbResult,
    options
  );
  const countryIso = cached.path(['country', 'iso_code']);
  benchLookup(
    'path country.iso',
    countryIso,
    ips,
    options.warmup,
    (lookup, ip) => lookup.get(ip),
    dbResult,
    options
  );
  benchMany(
    'getMany default cache',
    cached,
    ips,
    options.warmup,
    (reader, values) => reader.getMany(values),
    dbResult,
    options
  );
  await closeMaybe(cached);

  const largerCache = await benchOpen(
    'maxmind-rs cache:100k',
    () => maxmind.open(db, { cache: { max: 100_000 } }),
    dbResult,
    options
  );
  benchLookup(
    'get cache:100k',
    largerCache,
    ips,
    options.warmup,
    (reader, ip) => reader.get(ip),
    dbResult,
    options
  );
  benchMany(
    'getMany cache:100k',
    largerCache,
    ips,
    options.warmup,
    (reader, values) => reader.getMany(values),
    dbResult,
    options
  );
  await closeMaybe(largerCache);

  if (nodeMaxmind) {
    const nodeDefault = await benchOpen(
      'node-maxmind default cache',
      () => nodeMaxmind.open(db),
      dbResult,
      options
    );
    benchLookup(
      'node-maxmind get default cache',
      nodeDefault,
      ips,
      options.warmup,
      (reader, ip) => reader.get(ip),
      dbResult,
      options
    );

    const nodeLargerCache = await benchOpen(
      'node-maxmind cache:100k',
      () => nodeMaxmind.open(db, { cache: { max: 100_000 } }),
      dbResult,
      options
    );
    benchLookup(
      'node-maxmind get cache:100k',
      nodeLargerCache,
      ips,
      options.warmup,
      (reader, ip) => reader.get(ip),
      dbResult,
      options
    );
  }

  return dbResult;
}

function loadBaseline(filepath) {
  return JSON.parse(fs.readFileSync(filepath, 'utf8'));
}

function writeBaseline(filepath, results) {
  fs.writeFileSync(filepath, `${JSON.stringify(results, null, 2)}\n`);
}

function findBaselineDb(baseline, currentPath) {
  const dbs = baseline.dbs ?? [];
  return (
    dbs.find((db) => db.path === currentPath) ??
    dbs.find((db) => path.basename(db.path) === path.basename(currentPath))
  );
}

function findBenchmark(dbResult, label) {
  return dbResult?.benchmarks?.find((benchmark) => benchmark.label === label);
}

function compareWithBaseline(results, baseline, minRatio) {
  let failed = false;
  let compared = 0;

  console.error(`\nBaseline comparison, minimum ratio ${minRatio}`);
  for (const dbResult of results.dbs) {
    const baselineDb = findBaselineDb(baseline, dbResult.path);
    if (!baselineDb) {
      console.error(`SKIP ${dbResult.path}: no matching baseline database`);
      continue;
    }

    for (const benchmark of dbResult.benchmarks) {
      const baselineBenchmark = findBenchmark(baselineDb, benchmark.label);
      if (!baselineBenchmark || !baselineBenchmark.rate) {
        continue;
      }

      compared += 1;
      const ratio = benchmark.rate / baselineBenchmark.rate;
      const status = ratio < minRatio ? 'FAIL' : 'OK';
      if (ratio < minRatio) {
        failed = true;
      }
      console.error(
        `${status} ${path.basename(dbResult.path)} ${benchmark.label}: ` +
          `${Math.round(benchmark.rate).toLocaleString('en-US')}/s vs ` +
          `${Math.round(baselineBenchmark.rate).toLocaleString('en-US')}/s ` +
          `(${ratio.toFixed(3)}x)`
      );
    }
  }

  if (compared === 0) {
    throw new Error('Baseline comparison did not find any matching benchmarks');
  }

  if (failed) {
    process.exitCode = 1;
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const nodeMaxmind = options.compareNodeMaxmind ? loadNodeMaxmind() : null;
  const ips = makeIps(options.count);
  const results = {
    version: 1,
    generatedAt: new Date().toISOString(),
    count: options.count,
    warmup: options.warmup,
    dbs: [],
  };

  if (options.compareNodeMaxmind && !nodeMaxmind) {
    console.warn('Skipping node-maxmind comparison; ../node-maxmind/lib was not found');
  }

  for (const db of options.dbs) {
    results.dbs.push(await benchDatabase(db, options, ips, nodeMaxmind));
  }

  if (options.saveBaseline) {
    writeBaseline(options.saveBaseline, results);
  }

  if (options.baseline) {
    compareWithBaseline(results, loadBaseline(options.baseline), options.minRatio);
  }

  if (options.json) {
    console.log(JSON.stringify(results, null, 2));
  }
}

main().catch((error) => {
  console.error(error.stack || error.message);
  process.exitCode = 1;
});
