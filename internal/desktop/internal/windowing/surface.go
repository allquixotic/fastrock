// Package windowing contains display-independent storage and input helpers.
// Its tests never initialize a native window or graphics driver.
package windowing

import (
	"image"
	"image/draw"
)

// Capacity grows in geometric buckets and relinquishes substantially oversized
// surfaces. Small resize steps and modest shrink operations reuse storage.
func Capacity(current, needed image.Point) image.Point {
	needed.X, needed.Y = max(1, needed.X), max(1, needed.Y)
	if current.X >= needed.X && current.Y >= needed.Y && int64(current.X)*int64(current.Y) <= 4*int64(needed.X)*int64(needed.Y) {
		return current
	}
	grow := func(n int) int { return ((n + n/4 + 63) / 64) * 64 }
	return image.Pt(grow(needed.X), grow(needed.Y))
}

type Surface struct {
	Pixels *image.RGBA
	Damage image.Rectangle
	packed []byte
}

func (s *Surface) Ensure(view image.Point) bool {
	var old image.Point
	if s.Pixels != nil {
		old = s.Pixels.Rect.Size()
	}
	capacity := Capacity(old, view)
	if old == capacity {
		return false
	}
	next := image.NewRGBA(image.Rectangle{Max: capacity})
	if s.Pixels != nil {
		draw.Draw(next, next.Bounds(), s.Pixels, image.Point{}, draw.Src)
	}
	s.Pixels = next
	s.Damage = next.Bounds()
	return true
}

func (s *Surface) Upload(dp image.Point, source *image.RGBA, sr image.Rectangle) {
	dst := image.Rectangle{Min: dp, Max: dp.Add(sr.Size())}.Intersect(s.Pixels.Bounds())
	draw.Draw(s.Pixels, dst, source, sr.Min.Add(dst.Min.Sub(dp)), draw.Src)
	s.Damage = s.Damage.Union(dst)
}

// PackedDamage returns the one changed rectangle as tightly packed pixels for
// a single GPU subimage upload. Full-width rows need no staging copy.
func (s *Surface) PackedDamage() (image.Rectangle, []byte) {
	r := s.Damage.Intersect(s.Pixels.Bounds())
	if r.Empty() {
		return r, nil
	}
	n := r.Dx() * r.Dy() * 4
	start := s.Pixels.PixOffset(r.Min.X, r.Min.Y)
	if r.Dx()*4 == s.Pixels.Stride {
		return r, s.Pixels.Pix[start : start+n]
	}
	if cap(s.packed) < n || cap(s.packed) > max(64<<10, 4*n) {
		s.packed = make([]byte, n, n+n/4)
	} else {
		s.packed = s.packed[:n]
	}
	for y := 0; y < r.Dy(); y++ {
		copy(s.packed[y*r.Dx()*4:], s.Pixels.Pix[start+y*s.Pixels.Stride:start+y*s.Pixels.Stride+r.Dx()*4])
	}
	return r, s.packed
}
