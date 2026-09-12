'use strict';

const assert = require('node:assert/strict');
const childProcess = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const maxmind = require('..');
const dataDir = path.join(
  __dirname,
  '../../maxminddb-rust/test-data/test-data'
);
const cityDb = path.join(dataDir, 'GeoIP2-City-Test.mmdb');

test('bound lookups share reader state and ignore unrelated receivers', () => {
  const reader = new maxmind.Reader(fs.readFileSync(cityDb));
  const get = reader._get;
  try {
    for (const ip of [
      undefined,
      null,
      42,
      true,
      1n,
      Symbol('ip'),
      {},
      [],
      Buffer.alloc(0),
    ]) {
      assert.throws(() => reader.get(ip), {
        code: 'InvalidArg',
        message: 'IP address must be a string',
      });
    }
    const record = reader.get('81.2.69.142');
    assert.strictEqual(get.call({}, '81.2.69.142'), record);
    assert.strictEqual(reader._reader.get('81.2.69.142'), record);
    reader.clearCache();
    assert.notStrictEqual(get('81.2.69.142'), record);

    reader.load(
      fs.readFileSync(path.join(dataDir, 'MaxMind-DB-test-decoder.mmdb'))
    );
    assert.equal(get('1.1.1.1').uint16, 100);
    assert.strictEqual(get('1.1.1.1'), reader.get('1.1.1.1'));
  } finally {
    reader.close();
  }
  assert.throws(() => get('1.1.1.1'), /closed MaxMind DB/);
});

test('binding a lookup rejects invalid native receivers', () => {
  const reader = new maxmind.Reader(fs.readFileSync(cityDb));
  const iterator = reader.networks();
  try {
    const bind = reader._reader.bindGet;
    for (const receiver of [
      {},
      Object.create(Object.getPrototypeOf(reader._reader)),
      iterator._cursor,
    ]) {
      assert.throws(() => bind.call(receiver));
    }
    assert.equal(reader.get('81.2.69.142').country.iso_code, 'GB');
  } finally {
    iterator.close();
    reader.close();
  }
});

test('bound lookups retain their native reader and release it after collection', () => {
  const result = childProcess.spawnSync(
    process.execPath,
    [
      '--expose-gc',
      '-e',
      `
    const assert = require('node:assert/strict');
    const fs = require('node:fs');
    const maxmind = require('.');

    async function collected(reference) {
      for (let attempt = 0; attempt < 30; attempt += 1) {
        await new Promise(setImmediate);
        global.gc();
        await new Promise(setImmediate);
        if (reference.deref() === undefined) return;
      }
      assert.fail('Reader was retained after repeated garbage collection');
    }

    (async () => {
      let get;
      let wrapper;
      let native;
      (() => {
        const reader = new maxmind.Reader(fs.readFileSync(process.argv[1]));
        get = reader._get;
        wrapper = new WeakRef(reader);
        native = new WeakRef(reader._reader);
      })();
      await collected(wrapper);
      assert.notEqual(native.deref(), undefined);
      assert.equal(get('81.2.69.142').country.iso_code, 'GB');
      get = null;
      await collected(native);
    })().catch((error) => {
      console.error(error);
      process.exitCode = 1;
    });
  `,
      cityDb,
    ],
    {
      cwd: path.join(__dirname, '..'),
      encoding: 'utf8',
      timeout: 10_000,
    }
  );
  assert.equal(result.status, 0, result.stderr || result.error?.message);
});
