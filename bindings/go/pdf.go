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

type pdfState struct {
	mu     sync.Mutex
	handle *C.pydoccraft_pdf
}

// PDF owns an editable PDF. Close releases it; Copy creates an independent editor.
// Copies of the Go value share ownership and closing either closes both.
type PDF struct{ state *pdfState }

func pdfResult(h *C.pydoccraft_pdf, err error) (*PDF, error) {
	if err != nil {
		if h != nil {
			_, _ = take(C.pydoccraft_pdf_close(h))
		}
		return nil, err
	}
	if h == nil {
		return nil, fmt.Errorf("doccraft: native PDF missing")
	}
	return &PDF{&pdfState{handle: h}}, nil
}

// NewPDF creates a PDF with one page of the supplied size in points.
func NewPDF(width, height float64) (*PDF, error) {
	var h *C.pydoccraft_pdf
	_, err := take(C.pydoccraft_pdf_new(C.double(width), C.double(height), &h))
	return pdfResult(h, err)
}

// OpenPDF imports an existing PDF. Encrypted, signed and form PDFs may be unsupported.
func OpenPDF(data []byte, password *string) (*PDF, error) {
	var p []byte
	var has C.uint8_t
	var h *C.pydoccraft_pdf
	if password != nil {
		p = []byte(*password)
		has = 1
	}
	r := C.pydoccraft_pdf_open(ptr(data), C.size_t(len(data)), ptr(p), C.size_t(len(p)), has, &h)
	runtime.KeepAlive(data)
	runtime.KeepAlive(p)
	_, err := take(r)
	return pdfResult(h, err)
}
func (p *PDF) lock() (*pdfState, error) {
	if p == nil || p.state == nil {
		return nil, &Error{10, "PDF closed"}
	}
	s := p.state
	s.mu.Lock()
	if s.handle == nil {
		s.mu.Unlock()
		return nil, &Error{10, "PDF closed"}
	}
	return s, nil
}

// Close is idempotent and waits for any ongoing operation.
func (p *PDF) Close() error {
	if p == nil || p.state == nil {
		return nil
	}
	s := p.state
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.handle == nil {
		return nil
	}
	h := s.handle
	s.handle = nil
	_, err := take(C.pydoccraft_pdf_close(h))
	return err
}

// Copy creates an independent editor.
func (p *PDF) Copy() (*PDF, error) {
	s, err := p.lock()
	if err != nil {
		return nil, err
	}
	defer s.mu.Unlock()
	var h *C.pydoccraft_pdf
	_, err = take(C.pydoccraft_pdf_copy(s.handle, &h))
	return pdfResult(h, err)
}
func (p *PDF) command(v map[string]any) ([]byte, error) {
	s, err := p.lock()
	if err != nil {
		return nil, err
	}
	defer s.mu.Unlock()
	b, err := json.Marshal(v)
	if err != nil {
		return nil, &Error{1, err.Error()}
	}
	r := C.pydoccraft_pdf_command(s.handle, ptr(b), C.size_t(len(b)))
	runtime.KeepAlive(b)
	return take(r)
}

// PageCount returns the number of pages.
func (p *PDF) PageCount() (int, error) {
	b, e := p.command(map[string]any{"op": "count"})
	if e != nil {
		return 0, e
	}
	return strconv.Atoi(string(b))
}

// PageSize returns the displayed page width and height in points. Negative indexes count from the end.
func (p *PDF) PageSize(page int) (float64, float64, error) {
	b, e := p.command(map[string]any{"op": "size", "page": page})
	if e != nil {
		return 0, 0, e
	}
	var v [2]float64
	if e = json.Unmarshal(b, &v); e != nil {
		return 0, 0, e
	}
	return v[0], v[1], nil
}

// AddPage appends a page and returns its zero-based index.
func (p *PDF) AddPage(width, height float64) (int, error) {
	b, e := p.command(map[string]any{"op": "add_page", "width": width, "height": height})
	if e != nil {
		return 0, e
	}
	return strconv.Atoi(string(b))
}

// DeletePage removes a page.
func (p *PDF) DeletePage(page int) error {
	_, e := p.command(map[string]any{"op": "delete_page", "page": page})
	return e
}

// RotatePage rotates a page by a multiple of 90 degrees.
func (p *PDF) RotatePage(page int, degrees int) error {
	_, e := p.command(map[string]any{"op": "rotate", "page": page, "degrees": degrees})
	return e
}

// AddText adds black text at the top-left position in displayed page points.
func (p *PDF) AddText(page int, text string, x, y, size float64) error {
	if len(text) > 1000000 {
		return &Error{5, "PDF text too large"}
	}
	if !utf8.ValidString(text) {
		return &Error{1, "invalid UTF-8"}
	}
	_, e := p.command(map[string]any{"op": "text", "page": page, "text": text, "x": x, "y": y, "size": size})
	return e
}

// Save serializes the PDF to owned bytes.
func (p *PDF) Save() ([]byte, error) { return p.command(map[string]any{"op": "save"}) }
