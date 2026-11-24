'use strict';

const assert = require('node:assert/strict');
const childProcess = require('node:child_process');
const path = require('node:path');
const test = require('node:test');

const indexPath = path.join(__dirname, '..', 'index.js');

// Loads index.js in a child process with no native modules present and
// returns the candidate file names the loader tried, in order. A string
// `ldd` is returned as the contents of /usr/bin/ldd; null makes that read
// fail. `sharedObjects`, if given, replaces the process report's list.
function attemptedCandidates({ ldd, sharedObjects }) {
  const script = `
    const fs = require('node:fs');
    const readFileSync = fs.readFileSync;
    const ldd = ${JSON.stringify(ldd)};
    const sharedObjects = ${JSON.stringify(sharedObjects ?? null)};
    fs.existsSync = () => false;
    fs.readFileSync = (file, ...args) => {
      if (file !== '/usr/bin/ldd') return readFileSync(file, ...args);
      if (ldd === null) throw new Error('ENOENT: no such file or directory');
      return ldd;
    };
    if (sharedObjects !== null) {
      process.report.getReport = () => ({ header: {}, sharedObjects });
    }
    process.report.excludeNetwork = false;
    try {
      require(${JSON.stringify(indexPath)});
    } catch (err) {
      if (err.code !== 'ERR_MAXMINDDB_NATIVE_BINDING_NOT_FOUND') throw err;
      if (process.report.excludeNetwork !== false) {
        throw new Error('process.report.excludeNetwork was not restored');
      }
      process.stdout.write(err.message);
    }
  `;
  const output = childProcess.execFileSync(process.execPath, ['-e', script], {
    encoding: 'utf8',
  });
  return output
    .split('\n')
    .slice(1)
    .map((candidate) => path.basename(candidate));
}

const linuxWithMusl =
  process.platform === 'linux' && ['x64', 'arm64'].includes(process.arch);
const skip = !linuxWithMusl && 'needs Linux x64 or arm64';
const gnuFirst = () => [
  `index.linux-${process.arch}-gnu.node`,
  `index.linux-${process.arch}-musl.node`,
  'index.node',
];
const hostUsesMusl = () =>
  process.report
    .getReport()
    .sharedObjects.some(
      (file) => file.includes('libc.musl-') || file.includes('ld-musl-')
    );
const muslFirst = () => [
  `index.linux-${process.arch}-musl.node`,
  `index.linux-${process.arch}-gnu.node`,
  'index.node',
];

test('loader tries the glibc build first on glibc Linux', { skip }, () => {
  assert.deepEqual(
    attemptedCandidates({ ldd: 'ldd (GNU libc) 2.39' }),
    gnuFirst()
  );
});

test('loader tries the musl build first on musl Linux', { skip }, () => {
  assert.deepEqual(
    attemptedCandidates({ ldd: 'musl libc (x86_64)' }),
    muslFirst()
  );
});

test(
  'loader uses the process report when ldd is missing',
  { skip },
  async (t) => {
    await t.test('musl loader in shared objects', () => {
      assert.deepEqual(
        attemptedCandidates({
          ldd: null,
          sharedObjects: ['/lib/ld-musl-x86_64.so.1'],
        }),
        muslFirst()
      );
    });

    await t.test('glibc shared objects', () => {
      assert.deepEqual(
        attemptedCandidates({
          ldd: null,
          sharedObjects: ['/lib/x86_64-linux-gnu/libc.so.6'],
        }),
        gnuFirst()
      );
    });

    await t.test('no shared object information', () => {
      assert.deepEqual(
        attemptedCandidates({ ldd: null, sharedObjects: [] }),
        gnuFirst()
      );
    });

    await t.test('real process report', () => {
      // The child process sees the same report as this one, so the expected
      // order follows the libc of the machine running the tests.
      assert.deepEqual(
        attemptedCandidates({ ldd: null }),
        hostUsesMusl() ? muslFirst() : gnuFirst()
      );
    });
  }
);
