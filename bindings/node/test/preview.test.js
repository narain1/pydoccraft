'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const api = require('..');
const fixture = name => fs.readFileSync(path.join(__dirname, '../../../tests/data', name));

test('detects signatures and rejects invalid inputs', () => {
  assert.equal(api.detect(Buffer.from('%PDF-1.7\n')), 'pdf');
  assert.equal(api.detect(fixture('wordcraft-rectangle-stroke100.docx')), 'docx');
  assert.equal(api.detect(fixture('deckcraft-table-line-breaks.pptx')), 'pptx');
  assert.throws(() => api.detect(Buffer.from('unknown')), /unsupported:/);
  assert.throws(() => api.detect('not a buffer'), /invalid_argument:/);
  assert.throws(() => api.detect(), /invalid_argument:/);
});

test('converts DOCX to PDF and identifies the result', () => {
  const pdf = api.convert(fixture('wordcraft-rectangle-stroke100.docx'), 'docx', 'pdf');
  assert.ok(Buffer.isBuffer(pdf));
  assert.equal(api.detect(pdf), 'pdf');
});

test('rejects unsupported conversions and bad arguments', () => {
  assert.throws(() => api.convert(Buffer.from('x'), 'pdf', 'pptx'), /unsupported:/);
  assert.throws(() => api.convert(fixture('deckcraft-table-line-breaks.pptx'), 'powerpoint', 'pdf'), /unsupported:/);
  assert.throws(() => api.convert(Buffer.from('x'), 'pdf', 42), /invalid_argument:/);
  assert.throws(() => api.convert(Buffer.from('x'), 'pdf', 'docx', 'extra'), /invalid_argument:/);
});

test('rejects malformed documents and invalid buffer storage', () => {
  for (const value of [null, {}, new Uint8Array([1]), Buffer.alloc(0)]) {
    assert.throws(() => api.detect(value));
  }
  assert.throws(() => api.detect(Buffer.from('PK\x03\x04broken')));
  assert.throws(() => api.convert(Buffer.from('%PDF-1.7\ninvalid'), 'pdf', 'docx'));
  assert.throws(() => api.convert(Buffer.from('PK\x03\x04broken'), 'docx', 'pdf'));
  const shared = Buffer.from(new SharedArrayBuffer(16));
  assert.throws(() => api.detect(shared), /invalid_argument:/);
  assert.throws(() => api.convert(shared, 'pdf', 'docx'), /invalid_argument:/);
  assert.throws(() => api.convert(Buffer.from('x'), 'x'.repeat(1025), 'pdf'), /invalid_argument:/);
});

test('rejects oversized input before parsing', () => {
  const oversized = Buffer.alloc(128 * 1024 * 1024 + 1);
  assert.throws(() => api.detect(oversized), /resource_limit:/);
  assert.throws(() => api.convert(oversized, 'docx', 'pdf'), /resource_limit:/);
});

test('never enables PPTX export through supported aliases', () => {
  const input = fixture('deckcraft-table-line-breaks.pptx');
  for (const source of ['pptx', '.PPTX', 'powerpoint', '.POWERPOINT']) {
    assert.throws(() => api.convert(input, source, 'pdf'), /unsupported:/);
  }
});

test('extracts normalized JSON and renders PNG', () => {
  const input = fixture('wordcraft-rectangle-stroke100.docx');
  const payload = JSON.parse(api.extract(input, 'docx'));
  assert.ok(payload.pages.length > 0);
  assert.deepEqual(api.render(input, 'docx', 0, 72).subarray(0, 8), Buffer.from('89504e470d0a1a0a', 'hex'));
  assert.throws(() => api.render(input, 'docx', -1, 72), /invalid_argument:/);
  assert.throws(() => api.render(input, 'docx', 0.5, 72), /invalid_argument:/);
  assert.throws(() => api.render(input, 'docx', 0, NaN), /invalid_argument:/);
  assert.throws(() => api.render(input, 'epub', 0, 72), /unsupported:/);
  assert.throws(() => api.extract(Buffer.from('broken'), 'docx'));
  assert.throws(() => api.extract(Buffer.from(new SharedArrayBuffer(8)), 'pdf'), /invalid_argument:/);
});
