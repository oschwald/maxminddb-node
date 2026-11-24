'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const maxmind = require('..');

const dataDir = path.join(
  __dirname,
  '../../maxminddb-rust/test-data/test-data'
);
const metadataMarker = Buffer.from('\xab\xcd\xefMaxMind.com', 'latin1');
const paddingBytes = 16 * 1024 * 1024;
const readerCount = 24;

// Builds a valid database with unreferenced padding at the end of the data
// section, so each in-memory reader holds a large native allocation while
// its JavaScript wrapper stays small.
function paddedDatabase() {
  const database = fs.readFileSync(path.join(dataDir, 'GeoIP2-City-Test.mmdb'));
  const metadataStart = database.lastIndexOf(metadataMarker);
  assert(metadataStart > 0);
  return Buffer.concat([
    database.subarray(0, metadataStart),
    Buffer.alloc(paddingBytes),
    database.subarray(metadataStart),
  ]);
}

function nextTurn() {
  return new Promise((resolve) => setImmediate(resolve));
}

// Creates readers that are never closed or referenced, without calling
// gc(), and counts how many JavaScript wrappers and native readers the
// garbage collector reclaims. Without the external memory report V8 sees only
// small objects and collects none of them.
async function collectUnreferencedReaders(createReader) {
  const collected = { wrappers: 0, native: 0 };
  const registry = new FinalizationRegistry((kind) => {
    collected[kind] += 1;
  });
  for (let index = 0; index < readerCount; index += 1) {
    const reader = await createReader();
    assert.equal(reader.get('175.16.199.1').country.iso_code, 'CN');
    registry.register(reader, 'wrappers');
    registry.register(reader._reader, 'native');
    await nextTurn();
  }
  // Collection and cleanup callbacks run as later tasks. Wait for them, but
  // stop as soon as both kinds of object have been seen.
  const deadline = Date.now() + 5000;
  while (
    (collected.wrappers === 0 || collected.native === 0) &&
    Date.now() < deadline
  ) {
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  return collected;
}

function assertCollected(collected) {
  assert(
    collected.wrappers > 0,
    `expected some of ${readerCount} readers to be collected`
  );
  assert(
    collected.native > 0,
    `expected some of ${readerCount} native readers to be collected`
  );
}

test('unreferenced buffer readers are collected without close()', async () => {
  const database = paddedDatabase();
  assertCollected(
    await collectUnreferencedReaders(() => new maxmind.Reader(database))
  );
});

test('unreferenced MODE_MEMORY readers are collected without close()', async (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'maxminddb-external-'));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const file = path.join(dir, 'padded.mmdb');
  fs.writeFileSync(file, paddedDatabase());

  assertCollected(
    await collectUnreferencedReaders(() =>
      maxmind.open(file, { mode: maxmind.MODE_MEMORY })
    )
  );
});

test('network iterators keep reading after their in-memory reader closes', () => {
  const database = paddedDatabase();
  const expected = [...new maxmind.Reader(database).networks()].length;
  assert(expected > 1);

  // A page size of 1 makes every later network a native read after close().
  const reader = new maxmind.Reader(database);
  const networks = reader.networks({ pageSize: 1 });
  const first = networks.next();
  reader.close();

  assert.equal(first.done, false);
  let count = 1;
  for (const _ of networks) {
    count += 1;
  }
  assert.equal(count, expected);
});
