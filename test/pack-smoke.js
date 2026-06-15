'use strict';

const childProcess = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const root = path.resolve(__dirname, '..');
const npmExecPath = process.env.npm_execpath;

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

function npmInvocation(args) {
  if (npmExecPath) {
    return {
      command: process.execPath,
      args: [npmExecPath, ...args],
      options: {},
    };
  }

  return {
    command: process.platform === 'win32' ? 'npm.cmd' : 'npm',
    args,
    options: process.platform === 'win32' ? { shell: true } : {},
  };
}

function execNpm(args, options = {}) {
  const invocation = npmInvocation(args);
  return execFile(invocation.command, invocation.args, {
    ...invocation.options,
    ...options,
  });
}

function execNpmOutput(args, options = {}) {
  const invocation = npmInvocation(args);
  return execFileOutput(invocation.command, invocation.args, {
    ...invocation.options,
    ...options,
  });
}

const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), 'maxminddb-pack-'));
let tarball = null;

try {
  const packOutput = execNpmOutput(['pack', '--json']);
  const [packInfo] = JSON.parse(packOutput);
  if (!packInfo?.filename) {
    throw new Error(`Unexpected npm pack output: ${packOutput}`);
  }

  tarball = path.join(root, packInfo.filename);
  fs.writeFileSync(
    path.join(tmpdir, 'package.json'),
    JSON.stringify({ private: true }, null, 2)
  );

  execNpm(['install', '--ignore-scripts', '--no-audit', '--no-fund', tarball], {
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
