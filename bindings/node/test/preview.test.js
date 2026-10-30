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
  assert.throws(() => api.convert(Buffer.from('x'), 'pdf', 'docx', null, 'extra'), /invalid_argument:/);
});

test('password and clipping arguments are validated and forwarded', () => {
  const input = fixture('wordcraft-rectangle-stroke100.docx');
  assert.ok(JSON.parse(api.extract(input, 'docx', null)).pages.length);
  assert.throws(() => api.extract(input, 'docx', 42), /invalid_argument:/);
  assert.throws(() => api.convert(input, 'docx', 'pdf', 42), /invalid_argument:/);
  const png = api.render(input, 'docx', 0, 72, null, [0, 0, 20, 30]);
  assert.deepEqual(png.subarray(0, 8), Buffer.from('89504e470d0a1a0a', 'hex'));
  assert.equal(png.readUInt32BE(16), 20);
  assert.equal(png.readUInt32BE(20), 30);
  for (const clip of [[0, 0, 0, 1], [0, 0, Infinity, 1], [0, 0, 1], 'bad']) {
    assert.throws(() => api.render(input, 'docx', 0, 72, null, clip));
  }
});

test('session owns bytes and closes idempotently', () => {
  const input = fixture('wordcraft-rectangle-stroke100.docx');
  const session = api.open(input, 'docx');
  input.fill(0);
  assert.ok(JSON.parse(session.extract()).pages.length);
  assert.equal(session.render(0, 72, [0, 0, 10, 10]).readUInt32BE(16), 10);
  assert.throws(() => session.convert('pdf'), /unsupported:/);
  const snapshot = session.extract();
  session.close();
  session.close();
  assert.ok(JSON.parse(snapshot).pages.length);
  assert.throws(() => session.extract(), /closed:/);
  assert.throws(() => session.render(0, 72), /closed:/);
  assert.throws(() => session.convert('pdf'), /closed:/);
  assert.throws(() => api.open(Buffer.from(new SharedArrayBuffer(8)), 'docx'), /invalid_argument:/);
  assert.throws(() => api.open(Buffer.from('broken'), 'docx'));
  assert.throws(() => api.open(Buffer.alloc(0), 'epub'), /unsupported:/);
});

test('native handles reject foreign values and tolerate reentrant close', () => {
  const native = require('../doccraft.node');
  for (const value of [null, {}, Buffer.alloc(8), 1]) {
    assert.throws(() => native.sessionExtract(value), /invalid_argument:/);
    assert.throws(() => native.sessionClose(value), /invalid_argument:/);
  }
  const session = api.open(fixture('wordcraft-rectangle-stroke100.docx'), 'docx');
  const clip = [0, 0, 10, 10];
  Object.defineProperty(clip, 0, { get() { session.close(); return 0; } });
  assert.throws(() => session.render(0, 72, clip), /closed:/);
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
