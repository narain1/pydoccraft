package doccraft

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"image/png"
	"math"
	"os"
	"sync"
	"testing"
)

func TestDetectAndErrors(t *testing.T) {
	got, err := Detect([]byte("%PDF-1.7\n"))
	if err != nil || got != "pdf" {
		t.Fatalf("Detect = %q, %v", got, err)
	}
	_, err = Detect(nil)
	var native *Error
	if !errors.As(err, &native) || native.Code != 2 {
		t.Fatalf("unexpected error: %v", err)
	}
	_, err = Convert(nil, "pdf", "epub", nil)
	if !errors.As(err, &native) || native.Code != 2 {
		t.Fatalf("unexpected conversion error: %v", err)
	}
	_, err = Convert(nil, ".PowerPoint", "pdf", nil)
	if !errors.As(err, &native) || native.Code != 2 {
		t.Fatalf("PPTX conversion must be rejected: %v", err)
	}
}

func TestExtractAndRender(t *testing.T) {
	data, err := os.ReadFile("../../tests/data/wordcraft-rectangle-stroke100.docx")
	if err != nil {
		t.Fatal(err)
	}
	payload, err := Extract(data, "docx", nil)
	if err != nil || !json.Valid(payload) {
		t.Fatalf("Extract: %v", err)
	}
	var document struct {
		Format string            `json:"format"`
		Pages  []json.RawMessage `json:"pages"`
	}
	if err := json.Unmarshal(payload, &document); err != nil || document.Format != "docx" || len(document.Pages) == 0 {
		t.Fatalf("invalid payload: %s (%v)", payload, err)
	}
	png, err := Render(data, "docx", 0, 72, nil, &Clip{0, 0, 50, 50})
	if err != nil || !bytes.HasPrefix(png, []byte("\x89PNG\r\n\x1a\n")) {
		t.Fatalf("Render: %v", err)
	}
	image, err := pngDecode(png)
	if err != nil || image.Width != 50 || image.Height != 50 {
		t.Fatalf("clipped PNG: %+v, %v", image, err)
	}
	_, err = Render(data, "docx", -1, 72, nil, nil)
	var native *Error
	if !errors.As(err, &native) || native.Code != 1 {
		t.Fatalf("negative page: %v", err)
	}
	_, err = Render(data, "epub", 0, 72, nil, nil)
	if !errors.As(err, &native) || native.Code != 2 {
		t.Fatalf("EPUB render: %v", err)
	}
	_, err = Render(data, "docx", 0, math.NaN(), nil, nil)
	if err == nil {
		t.Fatal("NaN DPI accepted")
	}
	_, err = Extract(data, string([]byte{0xff}), nil)
	if !errors.As(err, &native) || native.Code != 1 {
		t.Fatalf("invalid UTF-8: %v", err)
	}
}

func pngDecode(data []byte) (struct{ Width, Height int }, error) {
	config, err := png.DecodeConfig(bytes.NewReader(data))
	return struct{ Width, Height int }{config.Width, config.Height}, err
}

func TestDOCXToPDF(t *testing.T) {
	data, err := os.ReadFile("../../tests/data/wordcraft-rectangle-stroke100.docx")
	if err != nil {
		t.Fatal(err)
	}
	pdf, err := Convert(data, "docx", "pdf", nil)
	if err != nil {
		t.Fatal(err)
	}
	kind, err := Detect(pdf)
	if err != nil || kind != "pdf" {
		t.Fatalf("converted output: %q, %v", kind, err)
	}
	payload, err := Extract(pdf, "pdf", nil)
	if err != nil || !json.Valid(payload) {
		t.Fatalf("reopen converted PDF: %v", err)
	}
}

func TestSessionOwnershipAndClose(t *testing.T) {
	data, err := os.ReadFile("../../tests/data/wordcraft-rectangle-stroke100.docx")
	if err != nil {
		t.Fatal(err)
	}
	s, payload, err := OpenSession(data, "docx", nil)
	if err != nil {
		t.Fatal(err)
	}
	for i := range data {
		data[i] = 0
	}
	image, err := s.Render(0, 72, &Clip{0, 0, 50, 50})
	if err != nil || !bytes.HasPrefix(image, []byte("\x89PNG\r\n\x1a\n")) {
		t.Fatalf("render: %v", err)
	}
	_, err = s.SearchFor(0, "hello")
	var native *Error
	if !errors.As(err, &native) || native.Code != 2 {
		t.Fatalf("search: %v", err)
	}
	if err = s.Close(); err != nil {
		t.Fatal(err)
	}
	if err = s.Close(); err != nil {
		t.Fatal(err)
	}
	_, err = s.Render(0, 72, nil)
	if !errors.As(err, &native) || native.Code != 10 {
		t.Fatalf("closed: %v", err)
	}
	_, err = s.SearchFor(0, "hello")
	if !errors.As(err, &native) || native.Code != 10 {
		t.Fatalf("closed search: %v", err)
	}
	if !json.Valid(payload) {
		t.Fatal("payload invalid after close")
	}
}

func TestSessionConcurrentClose(t *testing.T) {
	data, err := os.ReadFile("../../tests/data/wordcraft-rectangle-stroke100.docx")
	if err != nil {
		t.Fatal(err)
	}
	s, _, err := OpenSession(data, "docx", nil)
	if err != nil {
		t.Fatal(err)
	}
	var wg sync.WaitGroup
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			_, err := s.Render(0, 72, nil)
			var native *Error
			if err != nil && (!errors.As(err, &native) || native.Code != 10) {
				t.Errorf("render: %v", err)
			}
		}()
	}
	wg.Add(1)
	go func() {
		defer wg.Done()
		if err := s.Close(); err != nil {
			t.Errorf("close: %v", err)
		}
	}()
	wg.Wait()
}

func TestSessionInvalidOpen(t *testing.T) {
	for _, kind := range []string{"epub", "unknown"} {
		s, payload, err := OpenSession(nil, kind, nil)
		var native *Error
		if s != nil || payload != nil || !errors.As(err, &native) || native.Code != 2 {
			t.Fatalf("open %s: %v", kind, err)
		}
	}
}

func sessionPDFFixture() []byte {
	stream := "BT /F1 18 Tf 20 50 Td (Hello session) Tj ET"
	objects := []string{
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
		"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%s\nendstream", len(stream), stream),
	}
	var out bytes.Buffer
	out.WriteString("%PDF-1.4\n")
	offsets := make([]int, 0, len(objects))
	for i, object := range objects {
		offsets = append(offsets, out.Len())
		fmt.Fprintf(&out, "%d 0 obj\n%s\nendobj\n", i+1, object)
	}
	xref := out.Len()
	out.WriteString("xref\n0 6\n0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&out, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&out, "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", xref)
	return out.Bytes()
}

func TestPDFSessionSearch(t *testing.T) {
	s, _, err := OpenSession(sessionPDFFixture(), "pdf", nil)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	rects, err := s.SearchFor(0, "Hello")
	if err != nil || len(rects) == 0 {
		t.Fatalf("search: %v %v", rects, err)
	}
	_, err = s.SearchFor(-1, "Hello")
	var native *Error
	if !errors.As(err, &native) || native.Code != 1 {
		t.Fatalf("negative search: %v", err)
	}
	_, err = s.Render(0, math.NaN(), nil)
	if err == nil {
		t.Fatal("NaN DPI accepted")
	}
}
