'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const maxmind = require('..');

const dataDir = path.join(
  __dirname,
  '../../maxminddb-rust/test-data/test-data'
);

test('loads native binding', () => {
  assert.equal(maxmind.nativeVersion(), '0.1.0');
});

test('validates IP addresses', () => {
  assert.equal(maxmind.validate('64.4.4.4'), true);
  assert.equal(maxmind.validate('2001:4860:0:1001::3004:ef68'), true);
  assert.equal(maxmind.validate('whhaaaazza'), false);
});

test('opens database and looks up records', async () => {
  const reader = await maxmind.open(path.join(dataDir, 'GeoIP2-City-Test.mmdb'));

  assert.equal(reader.metadata.databaseType, 'GeoIP2-City');
  assert(reader.metadata.buildEpoch instanceof Date);
  assert.equal(reader.get('1.1.1.1'), null);
  assert.equal(reader.get('175.16.199.1').country.iso_code, 'CN');
});

test('constructs reader from buffer', () => {
  const buffer = fs.readFileSync(path.join(dataDir, 'GeoIP2-City-Test.mmdb'));
  const reader = new maxmind.Reader(buffer);

  assert.equal(reader.get('175.16.199.1').country.iso_code, 'CN');
});

test('caches materialized records by data offset', async () => {
  const reader = await maxmind.open(path.join(dataDir, 'GeoIP2-City-Test.mmdb'));

  const first = reader.get('175.16.199.1');
  assert.strictEqual(reader.get('175.16.199.1'), first);
  assert.strictEqual(reader.getWithPrefixLength('175.16.199.1')[0], first);

  const uncached = await maxmind.open(path.join(dataDir, 'GeoIP2-City-Test.mmdb'), {
    cache: false,
  });
  assert.notStrictEqual(
    uncached.get('175.16.199.1'),
    uncached.get('175.16.199.1')
  );
});

test('rejects invalid cache sizes', () => {
  const buffer = fs.readFileSync(path.join(dataDir, 'GeoIP2-City-Test.mmdb'));
  assert.throws(
    () => new maxmind.Reader(buffer, { cache: { max: 0 } }),
    /positive 32-bit integer/
  );
});

test('returns prefix length', async () => {
  const reader = await maxmind.open(
    path.join(dataDir, 'MaxMind-DB-test-ipv4-24.mmdb')
  );

  assert.deepEqual(reader.getWithPrefixLength('1.1.1.3'), [
    { ip: '1.1.1.2' },
    31,
  ]);
});

test('looks up selected paths', async () => {
  const reader = await maxmind.open(path.join(dataDir, 'GeoIP2-City-Test.mmdb'));

  assert.equal(reader.getPath('175.16.199.1', ['country', 'iso_code']), 'CN');
  assert.equal(
    reader.getPath('81.2.69.142', ['subdivisions', 0, 'iso_code']),
    'ENG'
  );
  assert.equal(
    reader.getPath('81.2.69.142', ['subdivisions', -1, 'iso_code']),
    'ENG'
  );
  assert.equal(reader.getPath('1.1.1.1', ['country', 'iso_code']), null);
});

test('looks up batches', async () => {
  const reader = await maxmind.open(path.join(dataDir, 'GeoIP2-City-Test.mmdb'));
  const ips = ['1.1.1.1', '175.16.199.1', '81.2.69.142'];

  assert.deepEqual(
    reader.getMany(ips).map((record) => record?.country?.iso_code ?? null),
    [null, 'CN', 'GB']
  );
  assert.deepEqual(reader.getManyPath(ips, ['country', 'iso_code']), [
    null,
    'CN',
    'GB',
  ]);
});

test('iterates networks within a CIDR', async () => {
  const reader = await maxmind.open(path.join(dataDir, 'GeoIP2-City-Test.mmdb'));
  const records = reader.within('81.2.69.142/31');

  assert(records.length > 0);
  assert.deepEqual(records[0], [
    '81.2.69.142/31',
    reader.get('81.2.69.142'),
  ]);
});

test('paginates networks within a CIDR', async () => {
  const reader = await maxmind.open(path.join(dataDir, 'GeoIP2-City-Test.mmdb'));
  const records = reader.within('81.2.69.0/24');

  assert(records.length > 1);

  const firstPage = reader.withinPage('81.2.69.0/24', { limit: 1 });
  assert.deepEqual(firstPage.records, records.slice(0, 1));
  assert.equal(firstPage.nextOffset, 1);

  const secondPage = reader.withinPage('81.2.69.0/24', {
    limit: 1,
    offset: firstPage.nextOffset,
  });
  assert.deepEqual(secondPage.records, records.slice(1, 2));

  assert.throws(
    () => reader.withinPage('81.2.69.0/24', { limit: 0 }),
    /positive 32-bit integer/
  );
});

test('closes reader', async () => {
  const reader = await maxmind.open(path.join(dataDir, 'GeoIP2-City-Test.mmdb'));

  assert.equal(reader.closed, false);
  reader.close();
  assert.equal(reader.closed, true);
  assert.throws(() => reader.get('81.2.69.142'), /closed MaxMind DB/);
});

test('keeps legacy API errors', () => {
  assert.throws(() => maxmind.init(), /Maxmind v2 module has changed API/);
  assert.throws(() => maxmind.openSync(), /Maxmind v2 module has changed API/);
});

test('rejects gzip files in open', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'maxmind-rs-'));
  const gzipPath = path.join(dir, 'db.mmdb.gz');
  fs.writeFileSync(gzipPath, Buffer.from([0x1f, 0x8b, 0x08, 0x00]));

  await assert.rejects(
    () => maxmind.open(gzipPath),
    /passing in a file in gzip format/
  );
});

test('unwatches database files on close', async () => {
  const originalWatchFile = fs.watchFile;
  const originalUnwatchFile = fs.unwatchFile;
  const watched = [];
  const unwatched = [];
  const dbPath = path.join(dataDir, 'GeoIP2-City-Test.mmdb');

  fs.watchFile = (filepath, options, listener) => {
    watched.push({ filepath, options, listener });
  };
  fs.unwatchFile = (filepath, listener) => {
    unwatched.push({ filepath, listener });
  };

  try {
    const reader = await maxmind.open(dbPath, { watchForUpdates: true });

    assert.equal(watched.length, 1);
    assert.equal(watched[0].filepath, dbPath);

    reader.close();
    reader.close();

    assert.equal(unwatched.length, 1);
    assert.equal(unwatched[0].filepath, dbPath);
    assert.strictEqual(unwatched[0].listener, watched[0].listener);
  } finally {
    fs.watchFile = originalWatchFile;
    fs.unwatchFile = originalUnwatchFile;
  }
});
