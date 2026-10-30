package doccraft

/*
#cgo CFLAGS: -I${SRCDIR}/native
#include "pydoccraft.h"
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"runtime"
	"strconv"
	"sync"
	"unicode/utf8"
)

// EPUB is an editable, in-memory EPUB. Do not copy it; call Copy instead.
// All methods serialize with Close. Call Close to release native memory.
type EPUB struct {
	mu     sync.Mutex
	handle *C.pydoccraft_epub
}

// NewEPUB creates an empty EPUB. Metadata and a chapter are required before Save.
func NewEPUB() (*EPUB, error) {
	var h *C.pydoccraft_epub
	_, err := take(C.pydoccraft_epub_new(&h))
	if err != nil {
		if h != nil {
			_, _ = take(C.pydoccraft_epub_close(h))
		}
		return nil, err
	}
	if h == nil {
		return nil, fmt.Errorf("doccraft: native EPUB missing")
	}
	return &EPUB{handle: h}, nil
}

// Close releases the document; it is idempotent.
func (e *EPUB) Close() error {
	if e == nil {
		return nil
	}
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.handle == nil {
		return nil
	}
	h := e.handle
	e.handle = nil
	_, err := take(C.pydoccraft_epub_close(h))
	return err
}

// Copy returns an independent editable document that must also be closed.
func (e *EPUB) Copy() (*EPUB, error) {
	if e == nil {
		return nil, &Error{10, "EPUB closed"}
	}
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.handle == nil {
		return nil, &Error{10, "EPUB closed"}
	}
	var h *C.pydoccraft_epub
	_, err := take(C.pydoccraft_epub_copy(e.handle, &h))
	if err != nil {
		if h != nil {
			_, _ = take(C.pydoccraft_epub_close(h))
		}
		return nil, err
	}
	if h == nil {
		return nil, fmt.Errorf("doccraft: native EPUB copy missing")
	}
	return &EPUB{handle: h}, nil
}

// epubText rejects invalid UTF-8 rather than letting encoding/json replace it.
func epubText(s string, max int) error {
	if len(s) > max {
		return &Error{5, "EPUB binding input too large"}
	}
	if !utf8.ValidString(s) {
		return &Error{1, "invalid UTF-8"}
	}
	return nil
}
func (e *EPUB) command(v map[string]any, image []byte) ([]byte, error) {
	if e == nil {
		return nil, &Error{10, "EPUB closed"}
	}
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.handle == nil {
		return nil, &Error{10, "EPUB closed"}
	}
	b, err := json.Marshal(v)
	if err != nil {
		return nil, err
	}
	r := C.pydoccraft_epub_command(e.handle, ptr(b), C.size_t(len(b)), ptr(image), C.size_t(len(image)))
	runtime.KeepAlive(b)
	runtime.KeepAlive(image)
	return take(r)
}

// SetMetadata replaces all metadata atomically. Aggregate metadata is limited
// to 1 MiB by this binding, in addition to the native per-field limits.
func (e *EPUB) SetMetadata(title, identifier, language string, authors []string) error {
	if len(authors) > 1000 {
		return &Error{5, "EPUB authors exceed 1000"}
	}
	total := 0
	for _, field := range []struct {
		s     string
		limit int
	}{{title, 65536}, {identifier, 4096}, {language, 128}} {
		if err := epubText(field.s, field.limit); err != nil {
			return err
		}
		total += len(field.s)
	}
	for _, a := range authors {
		if err := epubText(a, 65536); err != nil {
			return err
		}
		total += len(a)
		if total > 1024*1024 {
			return &Error{5, "EPUB metadata exceeds 1 MiB"}
		}
	}
	if total > 1024*1024 {
		return &Error{5, "EPUB metadata exceeds 1 MiB"}
	}
	if authors == nil {
		authors = []string{}
	}
	_, err := e.command(map[string]any{"op": "metadata", "title": title, "identifier": identifier, "language": language, "authors": authors}, nil)
	return err
}

// AddChapter appends a chapter and returns its zero-based index.
func (e *EPUB) AddChapter(title string) (int, error) {
	if err := epubText(title, 65536); err != nil {
		return 0, err
	}
	b, err := e.command(map[string]any{"op": "chapter", "text": title}, nil)
	if err != nil {
		return 0, err
	}
	return strconv.Atoi(string(b))
}

// ChapterCount returns the number of chapters.
func (e *EPUB) ChapterCount() (int, error) {
	b, err := e.command(map[string]any{"op": "count"}, nil)
	if err != nil {
		return 0, err
	}
	return strconv.Atoi(string(b))
}
func epubChapter(chapter int) error {
	if chapter < 0 {
		return &Error{1, "chapter must be non-negative"}
	}
	return nil
}

// AddHeading appends a heading at level 1 through 6.
func (e *EPUB) AddHeading(chapter int, text string, level int) error {
	if err := epubChapter(chapter); err != nil {
		return err
	}
	if err := epubText(text, 65536); err != nil {
		return err
	}
	if level < 1 || level > 6 {
		return &Error{1, "heading level must be between 1 and 6"}
	}
	_, err := e.command(map[string]any{"op": "heading", "chapter": chapter, "text": text, "level": level}, nil)
	return err
}

// AddParagraph appends plain text, escaping XHTML markup.
func (e *EPUB) AddParagraph(chapter int, text string) error {
	if err := epubChapter(chapter); err != nil {
		return err
	}
	if err := epubText(text, 65536); err != nil {
		return err
	}
	_, err := e.command(map[string]any{"op": "paragraph", "chapter": chapter, "text": text}, nil)
	return err
}

// AddLink appends a link. Only HTTP, HTTPS, mailto and safe fragments are accepted.
func (e *EPUB) AddLink(chapter int, text, href string) error {
	if err := epubChapter(chapter); err != nil {
		return err
	}
	if err := epubText(text, 65536); err != nil {
		return err
	}
	if err := epubText(href, 16384); err != nil {
		return err
	}
	_, err := e.command(map[string]any{"op": "link", "chapter": chapter, "text": text, "href": href}, nil)
	return err
}

// AddImage appends a PNG, JPEG or GIF resource. Image content is signature-checked,
// not fully decoded. The caller must not mutate data during the call.
func (e *EPUB) AddImage(chapter int, data []byte, mediaType, alt string) error {
	if err := epubChapter(chapter); err != nil {
		return err
	}
	if len(data) > 16*1024*1024 {
		return &Error{5, "EPUB image exceeds 16 MiB"}
	}
	if err := epubText(mediaType, 128); err != nil {
		return err
	}
	if err := epubText(alt, 65536); err != nil {
		return err
	}
	_, err := e.command(map[string]any{"op": "image", "chapter": chapter, "media_type": mediaType, "alt": alt}, data)
	return err
}

// Save returns independently owned EPUB bytes without closing or modifying the document.
func (e *EPUB) Save() ([]byte, error) { return e.command(map[string]any{"op": "save"}, nil) }
