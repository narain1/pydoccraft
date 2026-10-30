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

test('authors EPUB, reopens snapshots, and copies independently', () => {
  const book = api.createEpub();
  assert.throws(() => book.save(), /invalid_argument:/);
  book.setMetadata('Created book', 'urn:test:created', 'en', ['Author One']);
  const chapter = book.addChapter('Introduction');
  assert.equal(chapter, 0);
  book.addHeading(chapter, 'Welcome & hello', 1);
  book.addParagraph(chapter, 'Created paragraph café');
  book.addLink(chapter, 'Project', 'https://example.com');
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lXcAAAAASUVORK5CYII=', 'base64');
  book.addImage(chapter, png, 'image/png', 'Pixel');
  const clone = book.copy();
  clone.addChapter('Appendix');
  assert.equal(book.chapterCount(), 1);
  assert.equal(clone.chapterCount(), 2);
  const data = book.save();
  assert.equal(api.detect(data), 'epub');
  const extracted = JSON.stringify(JSON.parse(api.extract(data, 'epub')));
  for (const expected of ['Created book', 'Author One', 'Welcome & hello', 'Created paragraph café', 'https://example.com']) assert.ok(extracted.includes(expected), expected);
  assert.deepEqual(Buffer.from(JSON.parse(api.extract(data, 'epub')).pages[0].images[0].data, 'base64'), png);
  assert.equal(JSON.parse(api.extract(clone.save(), 'epub')).pages.length, 2);
  clone.close();
  book.close();
  book.close();
  assert.throws(() => book.save(), /closed:/);
  assert.throws(() => book.copy(), /closed:/);
  assert.equal(api.detect(data), 'epub');
});

test('EPUB authoring rejects invalid changes without mutation', () => {
  const book = api.createEpub();
  book.setMetadata('Original', 'urn:test', 'en', ['A']);
  const chapter = book.addChapter('One');
  book.addParagraph(chapter, 'unchanged');
  assert.throws(() => book.setMetadata('changed', 'urn:test', 'en', ['bad\u0001']), /invalid_argument:/);
  assert.throws(() => book.setMetadata('A & B', 'urn:test', 'en', ['A']), /unsupported:/);
  assert.throws(() => book.addChapter('A & B'), /unsupported:/);
  assert.throws(() => book.addHeading(chapter, 'bad', 7), /invalid_argument:/);
  assert.throws(() => book.addParagraph(chapter, '\ud800'), /invalid_argument:/);
  assert.throws(() => book.addLink(chapter, 'bad', 'javascript:alert(1)'), /unsupported:/);
  assert.throws(() => book.addImage(99, Buffer.from('89504e470d0a1a0a', 'hex'), 'image/png', 'bad'), /invalid_argument:/);
  assert.throws(() => book.addImage(chapter, Buffer.from('bad'), 'image/png', 'bad'), /invalid_argument:/);
  assert.throws(() => book.addImage(chapter, Buffer.alloc(16 * 1024 * 1024 + 1), 'image/png', ''), /resource_limit:/);
  assert.throws(() => book.addImage(chapter, Buffer.from(new SharedArrayBuffer(8)), 'image/png', ''), /invalid_argument:/);
  assert.throws(() => book.setMetadata('x', 'y', 'en', Array(1001).fill('a')), /resource_limit:/);
  const result = JSON.stringify(JSON.parse(api.extract(book.save(), 'epub')));
  assert.ok(result.includes('Original'));
  assert.ok(result.includes('unchanged'));
  assert.ok(!result.includes('javascript:'));
  book.close();
});

test('EPUB native handle validation and reentrant author getters', () => {
  const native = require('../doccraft.node');
  assert.throws(() => native.epubOp({}, 'save', null, null, null, null), /invalid_argument:/);
  const book = api.createEpub();
  const authors = ['A'];
  Object.defineProperty(authors, 0, { get() { book.close(); return 'A'; } });
  assert.throws(() => book.setMetadata('title', 'urn:test', 'en', authors), /closed:/);
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
