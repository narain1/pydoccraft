'use strict';

const native = require('./doccraft.node');

function count(actual, min, max) {
  if (actual < min || actual > max) throw new Error('invalid_argument:incorrect argument count');
}

function detect(data) {
  count(arguments.length, 1, 1);
  return native.detect(data);
}
function extract(data, filetype, password = null) {
  count(arguments.length, 2, 3);
  return native.extract(data, filetype, password);
}
function render(data, filetype, page, dpi, password = null, clip = null) {
  count(arguments.length, 4, 6);
  return native.render(data, filetype, page, dpi, password, clip);
}
function convert(data, source, target, password = null) {
  count(arguments.length, 3, 4);
  return native.convert(data, source, target, password);
}

function open(data, filetype, password = null) {
  count(arguments.length, 2, 3);
  const handle = native.sessionOpen(data, filetype, password);
  return Object.freeze({
    extract() { count(arguments.length, 0, 0); return native.sessionExtract(handle); },
    render(page, dpi, clip = null) { count(arguments.length, 2, 3); return native.sessionRender(handle, page, dpi, clip); },
    convert(target) { count(arguments.length, 1, 1); return native.sessionConvert(handle, target); },
    close() { count(arguments.length, 0, 0); return native.sessionClose(handle); },
  });
}

function wrapEpub(handle) {
  const op = (name, a = null, b = null, c = null, d = null) => native.epubOp(handle, name, a, b, c, d);
  return Object.freeze({
    setMetadata(title, identifier, language, authors) { count(arguments.length, 4, 4); return op('metadata', title, identifier, language, authors); },
    addChapter(title) { count(arguments.length, 1, 1); return op('chapter', title); },
    chapterCount() { count(arguments.length, 0, 0); return op('count'); },
    addHeading(chapter, text, level) { count(arguments.length, 3, 3); return op('heading', chapter, text, level); },
    addParagraph(chapter, text) { count(arguments.length, 2, 2); return op('paragraph', chapter, text); },
    addLink(chapter, text, href) { count(arguments.length, 3, 3); return op('link', chapter, text, href); },
    addImage(chapter, data, mediaType, alt) { count(arguments.length, 4, 4); return op('image', chapter, data, mediaType, alt); },
    save() { count(arguments.length, 0, 0); return op('save'); },
    copy() { count(arguments.length, 0, 0); return wrapEpub(op('copy')); },
    close() { count(arguments.length, 0, 0); return op('close'); },
  });
}
function createEpub() {
  count(arguments.length, 0, 0);
  return wrapEpub(native.epubNew());
}

module.exports = { detect, extract, render, convert, open, createEpub };
