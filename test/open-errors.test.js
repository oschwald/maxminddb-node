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

const fileModes = [maxmind.MODE_MMAP, maxmind.MODE_MEMORY];

function makeTempDir(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'maxminddb-open-errors-'));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

function fileError(code, filepath) {
  return (err) => {
    assert(err instanceof Error);
    assert.equal(err.code, code);
    assert.equal(err.path, filepath);
    assert.equal(typeof err.syscall, 'string');
    assert.match(err.message, new RegExp(`^${code}: `));
    assert(
      err.message.endsWith(`, ${err.syscall} '${filepath}'`),
      `message should name the syscall and path: ${err.message}`
    );
    assert.doesNotMatch(err.message, /os error/);
    return true;
  };
}

async function assertOpenFails(filepath, code) {
  for (const mode of fileModes) {
    await assert.rejects(
      () => maxmind.open(filepath, { mode }),
      fileError(code, filepath),
      `open() in ${mode} mode`
    );
    assert.throws(
      () => maxmind.Reader.open(filepath, { mode }),
      fileError(code, filepath),
      `Reader.open() in ${mode} mode`
    );
  }
}

test('open reports ENOENT with path for a missing file', async (t) => {
  const missing = path.join(makeTempDir(t), 'nope.mmdb');
  await assertOpenFails(missing, 'ENOENT');

  await assert.rejects(
    () => maxmind.open(missing),
    (err) => err.syscall === 'open'
  );
});

test('open reports EISDIR with path for a directory', async (t) => {
  const dir = makeTempDir(t);
  await assertOpenFails(dir, 'EISDIR');
});

test(
  'open reports EACCES with path for an unreadable file',
  {
    skip:
      process.platform === 'win32' || process.getuid?.() === 0
        ? 'file permissions are not enforced for this user'
        : false,
  },
  async (t) => {
    const unreadable = path.join(makeTempDir(t), 'unreadable.mmdb');
    fs.copyFileSync(path.join(dataDir, 'GeoIP2-City-Test.mmdb'), unreadable);
    fs.chmodSync(unreadable, 0o000);
    await assertOpenFails(unreadable, 'EACCES');
  }
);

test(
  'open reports EACCES for an unreadable directory',
  {
    skip:
      process.platform === 'win32' || process.getuid?.() === 0
        ? 'file permissions are not enforced for this user'
        : false,
  },
  async (t) => {
    const dir = path.join(makeTempDir(t), 'locked');
    fs.mkdirSync(dir, 0o000);
    try {
      await assertOpenFails(path.join(dir, 'db.mmdb'), 'EACCES');
    } finally {
      // Restore access so the temporary directory can be removed.
      fs.chmodSync(dir, 0o700);
    }
  }
);

test('watched reload of a missing file records ENOENT', async (t) => {
  const dbPath = path.join(makeTempDir(t), 'watched.mmdb');
  fs.copyFileSync(path.join(dataDir, 'GeoIP2-City-Test.mmdb'), dbPath);
  const reader = await maxmind.open(dbPath);
  t.after(() => reader.close());
  reader._watchFilepath = dbPath;
  fs.rmSync(dbPath);

  // waitForFile() unrefs its retry timers, so keep the event loop alive.
  const keepAlive = setInterval(() => {}, 1000);
  try {
    await reader._reloadWatchedFile(dbPath, maxmind.MODE_MMAP);
  } finally {
    clearInterval(keepAlive);
  }
  assert.equal(reader.lastReloadError.code, 'ENOENT');
  assert.equal(reader.lastReloadError.path, dbPath);
});

test('reload errors keep the code and path', async (t) => {
  const dir = makeTempDir(t);
  for (const mode of fileModes) {
    const dbPath = path.join(dir, `reload-${mode}.mmdb`);
    fs.copyFileSync(path.join(dataDir, 'GeoIP2-City-Test.mmdb'), dbPath);
    const reader = await maxmind.open(dbPath, { mode });
    t.after(() => reader.close());
    fs.rmSync(dbPath);

    assert.throws(() => reader.reload(), fileError('ENOENT', dbPath));
    assert.equal(reader.lastReloadError.code, 'ENOENT');
    assert.equal(reader.lastReloadError.path, dbPath);

    await assert.rejects(
      () => reader.reloadAsync(),
      fileError('ENOENT', dbPath)
    );
    assert.equal(reader.lastReloadError.code, 'ENOENT');
    assert.equal(reader.lastReloadError.path, dbPath);
  }
});
