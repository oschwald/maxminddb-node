'use strict';

const { existsSync } = require('node:fs');
const { join } = require('node:path');

const platformTriples = {
  linux: {
    x64: ['linux-x64-gnu', 'linux-x64-musl'],
    arm64: ['linux-arm64-gnu', 'linux-arm64-musl'],
    arm: ['linux-arm-gnueabihf', 'linux-arm-musleabihf'],
    ppc64: ['linux-ppc64-gnu'],
    s390x: ['linux-s390x-gnu'],
    riscv64: ['linux-riscv64-gnu'],
  },
  darwin: {
    x64: ['darwin-x64'],
    arm64: ['darwin-arm64'],
    universal: ['darwin-universal'],
  },
  win32: {
    x64: ['win32-x64-msvc', 'win32-x64-gnu'],
    arm64: ['win32-arm64-msvc'],
    ia32: ['win32-ia32-msvc'],
  },
  freebsd: {
    x64: ['freebsd-x64'],
  },
};

function nativeCandidates() {
  const triples = platformTriples[process.platform]?.[process.arch] ?? [];
  return [
    ...triples.map((triple) => `index.${triple}.node`),
    'index.node',
  ].map((name) => join(__dirname, name));
}

function loadNativeBinding() {
  const attempted = [];
  for (const candidate of nativeCandidates()) {
    attempted.push(candidate);
    if (existsSync(candidate)) {
      return require(candidate);
    }
  }

  const error = new Error(
    `Unable to load maxmind-rs native binding. Tried:\n${attempted.join('\n')}`
  );
  error.code = 'ERR_MAXMIND_RS_NATIVE_BINDING_NOT_FOUND';
  throw error;
}

module.exports = loadNativeBinding();
