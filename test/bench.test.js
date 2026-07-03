'use strict';

const assert = require('node:assert/strict');
const childProcess = require('node:child_process');
const path = require('node:path');
const test = require('node:test');

const dataDir = path.join(
  __dirname,
  '../../maxminddb-rust/test-data/test-data'
);
const benchPath = path.join(__dirname, '..', 'bench', 'lookup.js');
const testDb = path.join(dataDir, 'GeoIP2-City-Test.mmdb');

function runBenchmark(args) {
  return childProcess.spawnSync(process.execPath, [benchPath, ...args], {
    cwd: path.join(__dirname, '..'),
    encoding: 'utf8',
  });
}

test('benchmark rejects missing network CIDR option value', () => {
  const result = runBenchmark(['--network-cidr']);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /--network-cidr requires a CIDR/);
});

test('benchmark rejects missing network page size option value', () => {
  const result = runBenchmark(['--network-page-size']);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /--network-page-size must be a positive integer/);
});

test('benchmark can skip network iteration benchmarks', () => {
  const result = runBenchmark([
    '--json',
    '--count',
    '1',
    '--no-network-bench',
    testDb,
  ]);

  assert.equal(result.status, 0, result.stderr);
  const output = JSON.parse(result.stdout);
  const labels = output.dbs.flatMap((db) =>
    db.benchmarks.map((benchmark) => benchmark.label)
  );
  assert(!labels.includes('withinPages default cache'));
});
