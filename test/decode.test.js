'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const maxmind = require('..');

const dataDir = path.join(
  __dirname,
  '../../maxminddb-rust/test-data/test-data'
);

function openTestReader(database, options = {}) {
  return new maxmind.Reader(
    fs.readFileSync(path.join(dataDir, database)),
    options
  );
}

test('decodes all MaxMind DB value types', () => {
  const reader = openTestReader('MaxMind-DB-test-decoder.mmdb', {
    cache: false,
  });
  const record = reader.get('1.1.1.1');

  assert.equal(record.boolean, true);
  assert.equal(record.int32, -268435456);
  assert.equal(record.uint16, 100);
  assert.equal(record.uint32, 268435456);
  assert.equal(record.uint64, 1n << 60n);
  assert.equal(record.uint128, 1n << 120n);
  assert.equal(record.double, 42.123456);
  assert.equal(record.float, Math.fround(1.1));
  assert.equal(record.utf8_string, 'unicode! ☯ - ♫');
  assert.deepEqual(record.array, [1, 2, 3]);
  assert.deepEqual([...record.bytes], [0, 0, 0, 42]);
  assert(Buffer.isBuffer(record.bytes));
  assert.deepEqual(record.map, {
    mapX: {
      arrayX: [7, 8, 9],
      utf8_stringX: 'hello',
    },
  });
});

test('decodes zero and maximum numeric values', () => {
  const reader = openTestReader('MaxMind-DB-test-decoder.mmdb', {
    cache: false,
  });

  const zero = reader.get('::0.0.0.0');
  assert.deepEqual(zero.array, []);
  assert.equal(zero.boolean, false);
  assert.deepEqual([...zero.bytes], []);
  assert.equal(zero.double, 0);
  assert.equal(zero.float, 0);
  assert.equal(zero.int32, 0);
  assert.deepEqual(zero.map, {});
  assert.equal(zero.uint16, 0);
  assert.equal(zero.uint32, 0);
  assert.equal(zero.uint64, 0n);
  assert.equal(zero.uint128, 0n);
  assert.equal(zero.utf8_string, '');

  const max = reader.get('::255.255.255.255');
  assert.equal(max.double, Infinity);
  assert.equal(max.float, Infinity);
  assert.equal(max.int32, 2147483647);
  assert.equal(max.uint16, 65535);
  assert.equal(max.uint32, 4294967295);
  assert.equal(max.uint64, 18446744073709551615n);
  assert.equal(max.uint128, 340282366920938463463374607431768211455n);
});

test('decodes selected paths from mixed-type records', () => {
  const reader = openTestReader('MaxMind-DB-test-decoder.mmdb', {
    cache: false,
  });
  const uint128 = reader.path(['uint128']);

  assert.equal(reader.getPath('1.1.1.1', ['uint64']), 1n << 60n);
  assert.equal(uint128.get('1.1.1.1'), 1n << 120n);
  assert.deepEqual([...reader.getPath('1.1.1.1', ['bytes'])], [0, 0, 0, 42]);
  assert.equal(reader.getPath('1.1.1.1', ['array', -1]), 3);
  assert.equal(reader.getPath('1.1.1.1', ['map', 'mapX', 'arrayX', 1]), 8);
  assert.equal(reader.getPath('1.1.1.1', ['missing']), null);
});

test('decodes nested and string-only fixture records', () => {
  const nested = openTestReader('MaxMind-DB-test-nested.mmdb');
  assert.deepEqual(nested.get('1.1.1.1'), {
    map1: {
      map2: {
        array: [
          {
            map3: {
              a: 1,
              b: 2,
              c: 3,
            },
          },
        ],
      },
    },
  });
  assert.equal(
    nested.getPath('1.1.1.1', ['map1', 'map2', 'array', 0, 'map3', 'b']),
    2
  );

  const stringValues = openTestReader('MaxMind-DB-string-value-entries.mmdb');
  assert.equal(stringValues.get('1.1.1.1'), '1.1.1.1/32');
  assert.equal(stringValues.get('1.1.1.3'), '1.1.1.2/31');
  assert.equal(stringValues.get('1.1.1.5'), '1.1.1.4/30');
  assert.equal(stringValues.cacheStats().size, 0);
  assert.equal(stringValues.cacheStats().inserts, 0);
});

test('uses Node replacement semantics for malformed MMDB strings', () => {
  const source = fs.readFileSync(
    path.join(dataDir, 'MaxMind-DB-test-decoder.mmdb')
  );

  const invalidValueDatabase = Buffer.from(source);
  const stringOffset = invalidValueDatabase.indexOf(Buffer.from('unicode!'));
  assert.notEqual(stringOffset, -1);
  invalidValueDatabase[stringOffset] = 0xff;

  const invalidValueReader = new maxmind.Reader(invalidValueDatabase, {
    cache: false,
  });
  assert.equal(
    invalidValueReader.get('1.1.1.1').utf8_string,
    '�nicode! ☯ - ♫'
  );
  assert.equal(
    invalidValueReader.getPath('1.1.1.1', ['utf8_string']),
    '�nicode! ☯ - ♫'
  );

  const invalidKeyDatabase = Buffer.from(source);
  const keyOffset = invalidKeyDatabase.indexOf(Buffer.from('utf8_string'));
  assert.notEqual(keyOffset, -1);
  invalidKeyDatabase[keyOffset] = 0xff;

  const invalidKeyRecord = new maxmind.Reader(invalidKeyDatabase, {
    cache: false,
  }).get('1.1.1.1');
  assert.equal(invalidKeyRecord['�tf8_string'], 'unicode! ☯ - ♫');
  assert.equal(invalidKeyRecord.utf8_string, undefined);
});
