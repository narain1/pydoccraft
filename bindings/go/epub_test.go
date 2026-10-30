package doccraft

import (
	"archive/zip"
	"bytes"
	"encoding/json"
	"errors"
	"image"
	"image/png"
	"io"
	"strings"
	"sync"
	"testing"
)

func epubFixture(t *testing.T) *EPUB {
	t.Helper()
	e, err := NewEPUB()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := e.Close(); err != nil {
			t.Error(err)
		}
	})
	if err = e.SetMetadata("Title & more", "urn:test:123", "en", []string{"Author"}); err != nil {
		t.Fatal(err)
	}
	n, err := e.AddChapter("Chapter one")
	if err != nil || n != 0 {
		t.Fatalf("chapter: %d %v", n, err)
	}
	return e
}
func TestEPUBSaveReopenCopy(t *testing.T) {
	e := epubFixture(t)
	if err := e.AddHeading(0, "Heading", 2); err != nil {
		t.Fatal(err)
	}
	if err := e.AddParagraph(0, "Text <escaped> & more"); err != nil {
		t.Fatal(err)
	}
	if err := e.AddLink(0, "Link label", "https://example.com/?a=1&b=2"); err != nil {
		t.Fatal(err)
	}
	var img bytes.Buffer
	if err := png.Encode(&img, image.NewRGBA(image.Rect(0, 0, 1, 1))); err != nil {
		t.Fatal(err)
	}
	if err := e.AddImage(0, img.Bytes(), "image/png", "a picture"); err != nil {
		t.Fatal(err)
	}
	copyBook, err := e.Copy()
	if err != nil {
		t.Fatal(err)
	}
	defer copyBook.Close()
	if _, err = copyBook.AddChapter("Second"); err != nil {
		t.Fatal(err)
	}
	if count, err := e.ChapterCount(); err != nil || count != 1 {
		t.Fatalf("count %d %v", count, err)
	}
	data, err := e.Save()
	if err != nil {
		t.Fatal(err)
	}
	if err = e.Close(); err != nil {
		t.Fatal(err)
	}
	kind, err := Detect(data)
	if err != nil || kind != "epub" {
		t.Fatalf("detect %s %v", kind, err)
	}
	payload, err := Extract(data, "epub", nil)
	if err != nil || !json.Valid(payload) {
		t.Fatalf("reopen: %s %v", payload, err)
	}
	if !bytes.Contains(payload, []byte("Heading")) || !bytes.Contains(payload, []byte("Text <escaped>")) {
		t.Fatalf("missing text: %s", payload)
	}
	z, err := zip.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		t.Fatal(err)
	}
	var chapter string
	var foundImage bool
	for _, f := range z.File {
		if strings.HasSuffix(f.Name, "chapter-1.xhtml") {
			r, err := f.Open()
			if err != nil {
				t.Fatal(err)
			}
			b, err := io.ReadAll(r)
			r.Close()
			if err != nil {
				t.Fatal(err)
			}
			chapter = string(b)
		}
		if strings.HasSuffix(f.Name, "images/image-1.png") {
			foundImage = true
		}
	}
	if !strings.Contains(chapter, "Text &lt;escaped&gt; &amp; more") || !strings.Contains(chapter, "https://example.com/?a=1&amp;b=2") || !foundImage {
		t.Fatalf("chapter/image missing: %s %v", chapter, foundImage)
	}
	if _, err := copyBook.Save(); err != nil {
		t.Fatalf("copy after original close: %v", err)
	}
}
func TestEPUBFailureAtomicity(t *testing.T) {
	e := epubFixture(t)
	if err := e.AddParagraph(0, "before"); err != nil {
		t.Fatal(err)
	}
	for _, err := range []error{e.SetMetadata("changed", "urn:new", "en", []string{"bad\x01"}), e.AddLink(0, "bad", "javascript:alert(1)"), e.AddImage(0, []byte("bad"), "image/png", "bad"), e.AddParagraph(5, "bad"), e.AddHeading(0, "bad", 7), e.AddParagraph(0, string([]byte{0xff})), e.AddImage(0, nil, "image/svg+xml", "")} {
		if err == nil {
			t.Fatal("invalid operation succeeded")
		}
	}
	data, err := e.Save()
	if err != nil {
		t.Fatal(err)
	}
	payload, err := Extract(data, "epub", nil)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(payload, []byte("before")) || bytes.Contains(payload, []byte("bad")) {
		t.Fatalf("unexpected payload: %s", payload)
	}
	z, err := zip.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		t.Fatal(err)
	}
	for _, f := range z.File {
		if strings.Contains(f.Name, "images/image-") {
			t.Fatalf("failed image was added: %s", f.Name)
		}
		if strings.HasSuffix(f.Name, ".opf") {
			r, _ := f.Open()
			b, _ := io.ReadAll(r)
			r.Close()
			if !bytes.Contains(b, []byte("Title &amp; more")) || bytes.Contains(b, []byte("urn:new")) {
				t.Fatalf("metadata changed: %s", b)
			}
		}
	}
}
func TestEPUBCloseConcurrent(t *testing.T) {
	e := epubFixture(t)
	var wg sync.WaitGroup
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			err := e.AddParagraph(0, "concurrent")
			var native *Error
			if err != nil && (!errors.As(err, &native) || native.Code != 10) {
				t.Errorf("add: %v", err)
			}
		}()
	}
	wg.Add(1)
	go func() {
		defer wg.Done()
		if err := e.Close(); err != nil {
			t.Error(err)
		}
	}()
	wg.Wait()
	if err := e.Close(); err != nil {
		t.Fatal(err)
	}
	_, err := e.Save()
	var native *Error
	if !errors.As(err, &native) || native.Code != 10 {
		t.Fatalf("closed: %v", err)
	}
}
