'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const maxmind = require('..');

const dataRoot = path.join(__dirname, '../../maxminddb-rust/test-data');
const dataDir = path.join(dataRoot, 'test-data');
const badDataDir = path.join(dataRoot, 'bad-data');
const badDataError = {
  code: 'GenericFailure',
  message:
    "The MaxMind DB file's data section contains bad data (unknown data type or corrupt data)",
};
const modes = [maxmind.MODE_MMAP, maxmind.MODE_MEMORY, maxmind.MODE_BUFFER];
const ip = '1.2.3.4';
const metadataLimit = path.join(
  dataDir,
  'MaxMind-DB-test-metadata-payload-limit.mmdb'
);
const malformedDatabases = [
  'libmaxminddb/libmaxminddb-deep-array-nesting.mmdb',
  'libmaxminddb/libmaxminddb-deep-nesting.mmdb',
  'libmaxminddb/libmaxminddb-metadata-marker-only.mmdb',
  'libmaxminddb/libmaxminddb-offset-integer-overflow.mmdb',
  'libmaxminddb/libmaxminddb-oversized-array.mmdb',
  'libmaxminddb/libmaxminddb-oversized-map.mmdb',
  'maxminddb-golang/cyclic-data-structure.mmdb',
  'maxminddb-golang/invalid-bytes-length.mmdb',
  'maxminddb-golang/invalid-data-record-offset.mmdb',
  'maxminddb-golang/invalid-map-key-length.mmdb',
  'maxminddb-golang/invalid-string-length.mmdb',
  'maxminddb-golang/metadata-is-an-uint128.mmdb',
  'maxminddb-golang/unexpected-bytes.mmdb',
];

function lookupMalformedDatabase(reader) {
  try {
    reader.get('1.1.1.1');
    reader.get('128.0.0.1');
    // Traverse records that are not reached by either lookup.
    Array.from(reader.networks());
  } finally {
    reader.close();
  }
}

test('malformed databases raise bad-data errors through buffer readers', () => {
  for (const database of malformedDatabases) {
    assert.throws(
      () =>
        lookupMalformedDatabase(
          new maxmind.Reader(fs.readFileSync(path.join(badDataDir, database)))
        ),
      badDataError,
      database
    );
  }
});

for (const limit of ['value', 'payload']) {
  for (const cache of [false, { max: 10 }]) {
    test(`${limit} resource limits propagate through lookups (cache ${Boolean(cache)})`, () => {
      const reader = new maxmind.Reader(
        fs.readFileSync(
          path.join(dataDir, `MaxMind-DB-test-decoder-${limit}-limit-over.mmdb`)
        ),
        { cache }
      );
      const root = reader.path([]);
      try {
        const operations = {
          get: () => reader.get(ip),
          getWithPrefixLength: () => reader.getWithPrefixLength(ip),
          getPath: () => reader.getPath(ip, []),
          getPaths: () => reader.getPaths(ip, [[]]),
          getMany: () => reader.getMany([ip]),
          getManyPath: () => reader.getManyPath([ip], []),
          getManyPaths: () => reader.getManyPaths([ip], [[]]),
          'path.get': () => root.get(ip),
          'path.getMany': () => root.getMany([ip]),
        };
        for (const [name, operation] of Object.entries(operations)) {
          assert.throws(operation, badDataError, name);
        }
        assert.equal(reader.cacheStats().size, 0);

        reader.load(
          fs.readFileSync(path.join(dataDir, 'MaxMind-DB-test-decoder.mmdb'))
        );
        assert.equal(reader.get('1.1.1.1').uint16, 100);
        assert.equal(root.get('1.1.1.1').uint16, 100);
      } finally {
        root.close();
        reader.close();
      }
    });
  }

  for (const mode of modes) {
    test(`${limit} resource limits propagate through iteration in ${mode} mode`, async () => {
      const reader = await maxmind.open(
        path.join(dataDir, `MaxMind-DB-test-decoder-${limit}-limit-over.mmdb`),
        { mode }
      );
      try {
        assert.throws(() => reader.get(ip), badDataError);
        const iterators = [
          reader.networks(),
          reader.within('1.2.3.0/24'),
          reader.networksPath([]),
          reader.withinPath('1.2.3.0/24', []),
          reader.networkPages({ pageSize: 1 }),
          reader.withinPages('1.2.3.0/24', { pageSize: 1 }),
        ];
        for (const iterator of iterators) {
          try {
            assert.throws(() => iterator.next(), badDataError);
          } finally {
            iterator.return();
          }
        }
      } finally {
        reader.close();
      }
    });
  }
}

test('path navigation and decoding share the payload budget', () => {
  const reader = new maxmind.Reader(
    fs.readFileSync(
      path.join(dataDir, 'MaxMind-DB-test-decode-path-shared-budget.mmdb')
    )
  );
  const target = reader.path(['target']);
  try {
    for (const operation of [
      () => reader.getPath(ip, ['target']),
      () => reader.getPaths(ip, [['target']]),
      () => reader.getManyPath([ip], ['target']),
      () => reader.getManyPaths([ip], [['target']]),
      () => target.get(ip),
      () => target.getMany([ip]),
    ]) {
      assert.throws(operation, badDataError);
    }
  } finally {
    target.close();
    reader.close();
  }
});

test('records at the resource limits remain readable', () => {
  for (const fixture of [
    'value-limit',
    'value-limit-pointer-heavy',
    'payload-limit',
  ]) {
    const reader = new maxmind.Reader(
      fs.readFileSync(
        path.join(dataDir, `MaxMind-DB-test-decoder-${fixture}.mmdb`)
      ),
      { cache: false }
    );
    try {
      const record = reader.get(ip);
      assert(Array.isArray(record), fixture);
      if (fixture === 'payload-limit') {
        assert(record.every(Buffer.isBuffer));
        assert.equal(
          record.reduce((size, value) => size + value.length, 0),
          2 << 20
        );
      } else {
        assert.equal(
          record.flat(Infinity).length,
          fixture === 'value-limit' ? 65535 : 32768
        );
      }
      assert.deepEqual(reader.getPath(ip, []), record);
    } finally {
      reader.close();
    }
  }
});

test('metadata resource limits propagate through buffer opens and loads', () => {
  const database = fs.readFileSync(metadataLimit);
  assert.throws(() => new maxmind.Reader(database), badDataError);

  const reader = new maxmind.Reader(
    fs.readFileSync(path.join(dataDir, 'GeoIP2-City-Test.mmdb'))
  );
  try {
    const original = reader.get('81.2.69.142');
    assert.throws(() => reader.load(database), badDataError);
    assert.equal(reader.lastReloadError.message, badDataError.message);
    assert.strictEqual(reader.get('81.2.69.142'), original);
  } finally {
    reader.close();
  }
});

for (const mode of modes) {
  test(`malformed databases raise bad-data errors in ${mode} mode`, async () => {
    for (const database of malformedDatabases) {
      await assert.rejects(
        async () => {
          lookupMalformedDatabase(
            await maxmind.open(path.join(badDataDir, database), { mode })
          );
        },
        badDataError,
        database
      );
    }
  });

  test(`metadata resource limits propagate through opens and reloads in ${mode} mode`, async () => {
    await assert.rejects(
      () => maxmind.open(metadataLimit, { mode }),
      badDataError
    );

    const directory = fs.mkdtempSync(
      path.join(os.tmpdir(), 'maxminddb-resource-limit-')
    );
    const database = path.join(directory, 'database.mmdb');
    fs.copyFileSync(path.join(dataDir, 'GeoIP2-City-Test.mmdb'), database);
    const reader = await maxmind.open(database, { mode });
    try {
      const original = reader.get('81.2.69.142');
      // Replace the file without modifying the inode used by an active mmap.
      const replacement = path.join(directory, 'replacement.mmdb');
      fs.copyFileSync(metadataLimit, replacement);
      fs.renameSync(replacement, database);

      assert.throws(() => reader.reload(), badDataError);
      assert.equal(reader.lastReloadError.message, badDataError.message);
      assert.strictEqual(reader.get('81.2.69.142'), original);
      await assert.rejects(() => reader.reloadAsync(), badDataError);
      assert.equal(reader.lastReloadError.message, badDataError.message);
      assert.strictEqual(reader.get('81.2.69.142'), original);
    } finally {
      reader.close();
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });

  test(`empty containers at the end of metadata remain readable in ${mode} mode`, async () => {
    for (const container of ['array', 'map']) {
      const reader = await maxmind.open(
        path.join(
          badDataDir,
          'libmaxminddb',
          `libmaxminddb-empty-${container}-last-in-metadata.mmdb`
        ),
        { mode }
      );
      try {
        assert.deepEqual(reader.metadata.description, {});
        assert.deepEqual(reader.metadata.languages, []);
        assert.deepEqual(reader.get('1.1.1.1'), { ip: 'test' });
      } finally {
        reader.close();
      }
    }
  });
}

test('overflowing extended data types raise a database error', () => {
  const source = fs.readFileSync(
    path.join(dataDir, 'MaxMind-DB-string-value-entries.mmdb')
  );
  const valueOffset = source.indexOf(Buffer.from('1.1.1.1/32'));
  assert(valueOffset > 0);
  assert.equal(source[valueOffset - 1], 0x4a);
  for (let extendedType = 249; extendedType <= 255; extendedType += 1) {
    const database = Buffer.from(source);
    database[valueOffset - 1] = 0;
    database[valueOffset] = extendedType;
    const reader = new maxmind.Reader(database);
    try {
      assert.throws(() => reader.get('1.1.1.1'), badDataError);
      assert.throws(() => reader.getPath('1.1.1.1', []), badDataError);
    } finally {
      reader.close();
    }
  }
});
