'use strict';

const childProcess = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const root = path.resolve(__dirname, '..');
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';

function execFile(command, args, options = {}) {
  return childProcess.execFileSync(command, args, {
    cwd: root,
    stdio: 'inherit',
    ...options,
  });
}

function execFileOutput(command, args, options = {}) {
  return childProcess.execFileSync(command, args, {
    cwd: root,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'inherit'],
    ...options,
  });
}

const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), 'maxminddb-pack-'));
let tarball = null;

try {
  const packOutput = execFileOutput(npm, ['pack', '--json']);
  const [packInfo] = JSON.parse(packOutput);
  if (!packInfo?.filename) {
    throw new Error(`Unexpected npm pack output: ${packOutput}`);
  }

  tarball = path.join(root, packInfo.filename);
  fs.writeFileSync(
    path.join(tmpdir, 'package.json'),
    JSON.stringify({ private: true }, null, 2)
  );

  execFile(npm, ['install', '--ignore-scripts', '--no-audit', '--no-fund', tarball], {
    cwd: tmpdir,
  });

  execFile(
    process.execPath,
    [
      '-e',
      `
const maxminddb = require('maxminddb');
const pkg = require('maxminddb/package.json');
if (maxminddb.nativeVersion() !== pkg.version) {
  throw new Error('native version mismatch');
}
if (typeof maxminddb.open !== 'function') {
  throw new Error('missing open export');
}
`,
    ],
    { cwd: tmpdir }
  );
} finally {
  if (tarball) {
    fs.rmSync(tarball, { force: true });
  }
  fs.rmSync(tmpdir, { recursive: true, force: true });
}
