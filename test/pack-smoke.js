'use strict';

const childProcess = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const root = path.resolve(__dirname, '..');
const npmExecPath = process.env.npm_execpath;
const packageName = require(path.join(root, 'package.json')).name;

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
  const parsedPackOutput = JSON.parse(packOutput);
  const packInfo = Array.isArray(parsedPackOutput)
    ? parsedPackOutput.find((entry) => entry.name === packageName)
    : parsedPackOutput?.[packageName];
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

  const packageNameLiteral = JSON.stringify(packageName);
  const packageJsonLiteral = JSON.stringify(`${packageName}/package.json`);
  execFile(
    process.execPath,
    [
      '-e',
      `
const maxminddb = require(${packageNameLiteral});
const pkg = require(${packageJsonLiteral});
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

  // Typecheck a separate consumer project against the packed declarations
  // with library checking on and no ambient types, so the published
  // index.d.ts has to pull in everything it needs by itself. The consumer
  // gets the locked @types/node (an optional peer dependency) copied from
  // this checkout, so the check needs no network and stays reproducible.
  const rootModules = path.join(root, 'node_modules');
  const consumerModules = path.join(tmpdir, 'node_modules');
  const typesNodeDeps = Object.keys(
    require(path.join(rootModules, '@types/node/package.json')).dependencies ??
      {}
  );
  for (const dep of ['@types/node', ...typesNodeDeps]) {
    fs.cpSync(path.join(rootModules, dep), path.join(consumerModules, dep), {
      recursive: true,
    });
  }

  fs.writeFileSync(
    path.join(tmpdir, 'tsconfig.json'),
    JSON.stringify(
      {
        compilerOptions: {
          module: 'nodenext',
          moduleResolution: 'nodenext',
          target: 'es2022',
          strict: true,
          skipLibCheck: false,
          types: [],
          noEmit: true,
        },
        files: ['consumer.ts'],
      },
      null,
      2
    )
  );
  fs.writeFileSync(
    path.join(tmpdir, 'consumer.ts'),
    `import maxminddb = require(${packageNameLiteral});

export async function lookup(file: string): Promise<string | undefined> {
  const reader = await maxminddb.open<maxminddb.CityResponse>(file);
  return reader.get('8.8.8.8')?.country?.iso_code;
}

export function lookupInBuffer(
  database: Parameters<maxminddb.Reader['load']>[0],
): boolean {
  using reader = new maxminddb.Reader(database);
  return reader.get('8.8.8.8') !== null;
}
`
  );

  const tscPath = path.join(
    path.dirname(require.resolve('typescript/package.json', { paths: [root] })),
    'bin',
    'tsc'
  );
  execFile(process.execPath, [tscPath, '-p', tmpdir], { cwd: tmpdir });
} finally {
  if (tarball) {
    fs.rmSync(tarball, { force: true });
  }
  fs.rmSync(tmpdir, { recursive: true, force: true });
}
