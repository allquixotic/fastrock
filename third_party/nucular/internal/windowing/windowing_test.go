package windowing

import (
	"image"
	"image/color"
	"testing"
	"time"

	"golang.org/x/mobile/event/key"
	"golang.org/x/mobile/event/mouse"
	"golang.org/x/mobile/event/paint"
)

func TestV7SurfaceReusesAndShrinks(t *testing.T) {
	s := &Surface{}
	s.Ensure(image.Pt(800, 600))
	original := s.Pixels
	for n := 1; n <= 100; n++ {
		if s.Ensure(image.Pt(800+n, 600+n)) {
			t.Fatal("allocated during small resize", n)
		}
	}
	if s.Pixels != original {
		t.Fatal("buffer not reused")
	}
	s.Ensure(image.Pt(200, 150))
	if s.Pixels == original || s.Pixels.Rect.Dx() > 300 {
		t.Fatal("large buffer retained after shrink")
	}
}

func TestV4PackedDamagePreservesCoordinates(t *testing.T) {
	s := &Surface{}
	s.Ensure(image.Pt(100, 80))
	s.Damage = image.Rectangle{}
	src := image.NewRGBA(image.Rect(0, 0, 20, 20))
	for y := 0; y < 20; y++ {
		for x := 0; x < 20; x++ {
			src.SetRGBA(x, y, color.RGBA{uint8(x), uint8(y), 55, 255})
		}
	}
	s.Upload(image.Pt(7, 9), src, image.Rect(2, 3, 15, 17))
	r, pixels := s.PackedDamage()
	if r != image.Rect(7, 9, 20, 23) || len(pixels) != 13*14*4 {
		t.Fatal(r, len(pixels))
	}
	for y := 0; y < 14; y++ {
		for x := 0; x < 13; x++ {
			i := (y*13 + x) * 4
			if pixels[i] != uint8(x+2) || pixels[i+1] != uint8(y+3) {
				t.Fatal("wrong source offset", x, y)
			}
		}
	}
}

func TestV4EventsCoalesceAndPreserveBarriers(t *testing.T) {
	e := NewEvents()
	for i := 0; i < 10000; i++ {
		e.Send(mouse.Event{X: float32(i)})
	}
	e.Send(mouse.Event{X: 9999, Button: mouse.ButtonLeft, Direction: mouse.DirPress})
	e.Send(mouse.Event{X: 10001})
	e.Send(key.Event{Code: key.CodeA})
	for i := 0; i < 100; i++ {
		e.Send(paint.Event{})
	}
	for i := 0; i < 8; i++ {
		e.Send(Wheel{DeltaY: 0.125})
	}
	if e.count != 6 {
		t.Fatal("unbounded redundant events", e.count)
	}
	if got := e.Next().(mouse.Event); got.X != 9999 || got.Button != mouse.ButtonNone {
		t.Fatal(got)
	}
	if got := e.Next().(mouse.Event); got.Direction != mouse.DirPress {
		t.Fatal(got)
	}
	if got := e.Next().(mouse.Event); got.X != 10001 {
		t.Fatal(got)
	}
	if got := e.Next().(key.Event); got.Code != key.CodeA {
		t.Fatal(got)
	}
	if _, ok := e.Next().(paint.Event); !ok {
		t.Fatal("lost paint")
	}
	if got := e.Next().(Wheel); got.DeltaY != 1 {
		t.Fatal("lost fractional wheel", got)
	}
}

func TestV4EventsBackpressureRetainsKeys(t *testing.T) {
	e := NewEvents()
	for i := 0; i < 256; i++ {
		e.Send(i)
	}
	done := make(chan struct{})
	go func() { e.Send(256); close(done) }()
	select {
	case <-done:
		t.Fatal("overflow discarded event")
	case <-time.After(time.Millisecond):
	}
	for i := 0; i < 257; i++ {
		if got := e.Next(); got != i {
			t.Fatal(i, got)
		}
	}
	<-done
}
