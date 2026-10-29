package doccraft

import (
	"bytes"
	"encoding/json"
	"errors"
	"image/png"
	"math"
	"os"
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
