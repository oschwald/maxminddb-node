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
// Readers created before the test stops adding pressure. If the external
// memory report works, V8 collects some within the first few; if it does not,
// this bounds what each failing test allocates (about 1 GiB).
const maxReaders = 64;
// How long to wait for cleanup callbacks once maxReaders have been created.
const finalWaitMs = 5000;

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
//
// V8 collects on external memory pressure from incremental marking cycles it
// schedules itself, and readers created while a cycle is marking survive it.
// Whether a fixed batch of readers followed by an idle wait sees any
// collection therefore depends on when those cycles run, and on the Linux
// arm64 CI runner a batch of 24 sometimes saw none. Instead keep creating
// readers, which keeps up the pressure that starts new cycles, until both
// kinds of object have been seen to be collected, or until the cap is
// reached and a final wait has passed.
async function collectUnreferencedReaders(createReader) {
  const collected = { wrappers: 0, native: 0, created: 0 };
  const registry = new FinalizationRegistry((kind) => {
    collected[kind] += 1;
  });
  let deadline;
  while (collected.wrappers === 0 || collected.native === 0) {
    if (collected.created < maxReaders) {
      const reader = await createReader();
      assert.equal(reader.get('175.16.199.1').country.iso_code, 'CN');
      registry.register(reader, 'wrappers');
      registry.register(reader._reader, 'native');
      collected.created += 1;
      await nextTurn();
    } else {
      // Collection and cleanup callbacks run as later tasks.
      deadline ??= Date.now() + finalWaitMs;
      if (Date.now() >= deadline) {
        break;
      }
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  }
  return collected;
}

function assertCollected(collected) {
  const counts = `${collected.wrappers} wrappers and ${collected.native} native readers of ${collected.created} created`;
  assert(
    collected.wrappers > 0,
    `expected some readers to be collected (${counts})`
  );
  assert(
    collected.native > 0,
    `expected some native readers to be collected (${counts})`
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
