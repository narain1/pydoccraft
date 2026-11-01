// Package doccraft provides an experimental in-process document binding.
package doccraft

/*
#cgo CFLAGS: -I${SRCDIR}/native
#cgo LDFLAGS: -lpydoccraft_go
#include "pydoccraft.h"
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"runtime"
	"sync"
	"unsafe"
)

// Error is a native error. Code is stable for this ABI; Message
// contains the native diagnostic and may change.
type Error struct {
	Code    uint32
	Message string
}

func (e *Error) Error() string { return fmt.Sprintf("doccraft (%d): %s", e.Code, e.Message) }

func ptr(b []byte) *C.uint8_t {
	if len(b) == 0 {
		return nil
	}
	return (*C.uint8_t)(unsafe.Pointer(&b[0]))
}
func take(r C.pydoccraft_result) ([]byte, error) {
	defer C.pydoccraft_free(r.data)
	if uint64(r.data.len) > uint64(^uint(0)>>1) {
		return nil, fmt.Errorf("doccraft: native result too large")
	}
	data := append([]byte(nil), unsafe.Slice((*byte)(unsafe.Pointer(r.data.ptr)), int(r.data.len))...)
	if r.status != 0 {
		return nil, &Error{uint32(r.status), string(data)}
	}
	return data, nil
}

// Detect identifies a document from its bytes.
func Detect(data []byte) (string, error) {
	r := C.pydoccraft_detect(ptr(data), C.size_t(len(data)))
	runtime.KeepAlive(data)
	b, err := take(r)
	return string(b), err
}

// Convert converts document bytes. A nil password differs from an empty password.
func Convert(data []byte, source, target string, password *string) ([]byte, error) {
	s, t := []byte(source), []byte(target)
	var p []byte
	var has C.uint8_t
	if password != nil {
		p = []byte(*password)
		has = 1
	}
	r := C.pydoccraft_convert(ptr(data), C.size_t(len(data)), ptr(s), C.size_t(len(s)),
		ptr(t), C.size_t(len(t)), ptr(p), C.size_t(len(p)), has)
	runtime.KeepAlive(data)
	runtime.KeepAlive(s)
	runtime.KeepAlive(t)
	runtime.KeepAlive(p)
	return take(r)
}

// Extract returns the owned normalized document as JSON. EPUB extraction is
// supported alongside PDF, DOCX, and PPTX; see the native API for its schema.
func Extract(data []byte, filetype string, password *string) ([]byte, error) {
	f := []byte(filetype)
	var p []byte
	var has C.uint8_t
	if password != nil {
		p = []byte(*password)
		has = 1
	}
	r := C.pydoccraft_extract(ptr(data), C.size_t(len(data)), ptr(f), C.size_t(len(f)), ptr(p), C.size_t(len(p)), has)
	runtime.KeepAlive(data)
	runtime.KeepAlive(f)
	runtime.KeepAlive(p)
	return take(r)
}

// Clip is a rectangle in displayed page points.
type Clip struct{ X, Y, Width, Height float64 }

// Render returns PNG bytes for a zero-based page. A nil clip renders the full page.
func Render(data []byte, filetype string, page int, dpi float64, password *string, clip *Clip) ([]byte, error) {
	if page < 0 {
		return nil, &Error{1, "page must be non-negative"}
	}
	f := []byte(filetype)
	var p []byte
	var has, clipped C.uint8_t
	var x, y, width, height C.double
	if password != nil {
		p = []byte(*password)
		has = 1
	}
	if clip != nil {
		clipped = 1
		x = C.double(clip.X)
		y = C.double(clip.Y)
		width = C.double(clip.Width)
		height = C.double(clip.Height)
	}
	r := C.pydoccraft_render(ptr(data), C.size_t(len(data)), ptr(f), C.size_t(len(f)), ptr(p), C.size_t(len(p)), has, C.size_t(page), C.double(dpi), clipped, x, y, width, height)
	runtime.KeepAlive(data)
	runtime.KeepAlive(f)
	runtime.KeepAlive(p)
	return take(r)
}

// Session owns a native renderer. It must be closed and must not be copied.
// Methods are safe for concurrent use; Close waits for an active operation.
type Session struct {
	mu     sync.Mutex
	handle *C.pydoccraft_session
}

// OpenSession opens a PDF, DOCX or PPTX renderer and returns its independent
// normalized JSON payload. The caller must close the session. EPUB is unsupported.
func OpenSession(data []byte, filetype string, password *string) (*Session, []byte, error) {
	f := []byte(filetype)
	var p []byte
	var has C.uint8_t
	if password != nil {
		p = []byte(*password)
		has = 1
	}
	var h *C.pydoccraft_session
	r := C.pydoccraft_session_open(ptr(data), C.size_t(len(data)), ptr(f), C.size_t(len(f)), ptr(p), C.size_t(len(p)), has, &h)
	runtime.KeepAlive(data)
	runtime.KeepAlive(f)
	runtime.KeepAlive(p)
	payload, err := take(r)
	if err != nil {
		if h != nil {
			_, _ = take(C.pydoccraft_session_close(h))
		}
		return nil, nil, err
	}
	if h == nil {
		return nil, nil, fmt.Errorf("doccraft: native session missing")
	}
	return &Session{handle: h}, payload, nil
}

// Close releases native state. It is idempotent, including after a close error.
func (s *Session) Close() error {
	if s == nil {
		return nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.handle == nil {
		return nil
	}
	h := s.handle
	s.handle = nil
	_, err := take(C.pydoccraft_session_close(h))
	return err
}

// Render returns PNG bytes for a zero-based page.
func (s *Session) Render(page int, dpi float64, clip *Clip) ([]byte, error) {
	if s == nil {
		return nil, &Error{10, "session closed"}
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.handle == nil {
		return nil, &Error{10, "session closed"}
	}
	if page < 0 {
		return nil, &Error{1, "page must be non-negative"}
	}
	var clipped C.uint8_t
	var x, y, width, height C.double
	if clip != nil {
		clipped = 1
		x = C.double(clip.X)
		y = C.double(clip.Y)
		width = C.double(clip.Width)
		height = C.double(clip.Height)
	}
	return take(C.pydoccraft_session_render(s.handle, C.size_t(page), C.double(dpi), clipped, x, y, width, height))
}

// SearchFor returns displayed-page rectangles [x0, y0, x1, y1]. Only PDF
// sessions support searching.
func (s *Session) SearchFor(page int, query string) ([][4]float32, error) {
	if s == nil {
		return nil, &Error{10, "session closed"}
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.handle == nil {
		return nil, &Error{10, "session closed"}
	}
	if page < 0 {
		return nil, &Error{1, "page must be non-negative"}
	}
	q := []byte(query)
	r := C.pydoccraft_session_search(s.handle, C.size_t(page), ptr(q), C.size_t(len(q)))
	runtime.KeepAlive(q)
	data, err := take(r)
	if err != nil {
		return nil, err
	}
	var rects [][4]float32
	if err := json.Unmarshal(data, &rects); err != nil {
		return nil, fmt.Errorf("doccraft: invalid search result: %w", err)
	}
	return rects, nil
}
