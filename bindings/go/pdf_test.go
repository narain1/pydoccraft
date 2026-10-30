package doccraft

import (
	"bytes"
	"errors"
	"sync"
	"testing"
)

func TestPDFAuthoring(t *testing.T) {
	p, e := NewPDF(300, 400)
	if e != nil {
		t.Fatal(e)
	}
	defer p.Close()
	if n, e := p.AddPage(200, 250); e != nil || n != 1 {
		t.Fatalf("page: %d %v", n, e)
	}
	if e = p.AddText(0, "Hello PDF", 20, 20, 12); e != nil {
		t.Fatal(e)
	}
	b, e := p.Save()
	if e != nil {
		t.Fatal(e)
	}
	x, e := Extract(b, "pdf", nil)
	if e != nil || !bytes.Contains(x, []byte("Hello PDF")) {
		t.Fatalf("extract: %s %v", x, e)
	}
	q, e := OpenPDF(b, nil)
	if e != nil {
		t.Fatal(e)
	}
	defer q.Close()
	if n, e := q.PageCount(); e != nil || n != 2 {
		t.Fatalf("count: %d %v", n, e)
	}
	c, e := q.Copy()
	if e != nil {
		t.Fatal(e)
	}
	defer c.Close()
	if e = c.DeletePage(1); e != nil {
		t.Fatal(e)
	}
	if n, _ := q.PageCount(); n != 2 {
		t.Fatal("copy changed original")
	}
	alias := *q
	if e = alias.Close(); e != nil {
		t.Fatal(e)
	}
	_, e = q.Save()
	var native *Error
	if !errors.As(e, &native) || native.Code != 10 {
		t.Fatalf("closed: %v", e)
	}
}
func TestPDFConcurrentClose(t *testing.T) {
	p, e := NewPDF(300, 400)
	if e != nil {
		t.Fatal(e)
	}
	var wg sync.WaitGroup
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < 10; j++ {
				_, _ = p.PageCount()
			}
		}()
	}
	wg.Add(1)
	go func() { defer wg.Done(); _ = p.Close() }()
	wg.Wait()
}
