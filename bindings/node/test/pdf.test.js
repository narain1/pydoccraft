'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const api = require('..');
const native = require('../doccraft.node');

test('PDF authoring, independent copy, save and reopen', () => {
  const pdf = api.createPdf(300, 400);
  assert.equal(pdf.pageCount(), 1);
  assert.equal(pdf.addPage(200, 250), 1);
  assert.deepEqual(pdf.pageSize(-1), [200, 250]);
  pdf.addText(0, 'Hello Node PDF', 20, 20);
  const copy = pdf.copy();
  copy.addText(1, 'Copy only', 20, 20);
  const saved = pdf.save();
  assert.match(api.extract(saved, 'pdf'), /Hello Node PDF/);
  assert.doesNotMatch(api.extract(saved, 'pdf'), /Copy only/);
  assert.match(api.extract(copy.save(), 'pdf'), /Copy only/);
  const reopened = api.openPdf(saved);
  assert.equal(reopened.pageCount(), 2);
  reopened.addText(1, 'Reopened', 20, 20);
  assert.match(api.extract(reopened.save(), 'pdf'), /Reopened/);
  pdf.close(); pdf.close();
  assert.throws(() => pdf.save(), /closed/);
  assert.equal(copy.pageCount(), 2);
  copy.close(); reopened.close();
});

test('PDF input validation and native handle tags', () => {
  assert.throws(() => api.createPdf(-1, 400), /invalid_argument:/);
  const pdf = api.createPdf();
  assert.throws(() => pdf.pageSize(99), /page index/);
  assert.throws(() => pdf.addText(0, 'x', Infinity, 1), /invalid_argument:/);
  assert.throws(() => pdf.addText(0, '\ud800', 1, 1), /invalid_argument:/);
  assert.throws(() => api.openPdf(Buffer.from('not pdf')), /malformed:/);
  const epub = native.epubNew();
  assert.throws(() => native.pdfOp(epub, 'count', null, null, null, null, null), /invalid_argument:/);
  assert.throws(() => native.pdfOp({}, 'count', null, null, null, null, null), /invalid_argument:/);
  assert.throws(() => native.pdfOp(native.pdfNew(100, 100), 'shapes', null, null, null, null, null), /unsupported:/);
  pdf.close();
});
