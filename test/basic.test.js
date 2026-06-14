'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');

const maxmind = require('..');

test('loads native binding', () => {
  assert.equal(maxmind.nativeVersion(), '0.1.0');
});

