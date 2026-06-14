'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const fs = require('node:fs');
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
