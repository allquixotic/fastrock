package nucular

import (
	"image"
	"math"

	"github.com/golang/freetype/raster"
	"golang.org/x/image/math/fixed"
)

type shapeMaskKey struct{ width, height, corner int }
type shapeMaskCache struct {
	entries map[shapeMaskKey]*image.Alpha
	order   []shapeMaskKey
	bytes   int
}

// corner == -1 denotes an ellipse. Corners 0..3 start at top-left and
// proceed clockwise. The per-window cache is bounded by entries and bytes;
// unusually large shapes use the vector fallback without retaining a mask.
func (c *shapeMaskCache) get(width, height, corner int) *image.Alpha {
	if width <= 0 || height <= 0 || width > 512 || height > 512 {
		return nil
	}
	k := shapeMaskKey{width, height, corner}
	if m := c.entries[k]; m != nil {
		return m
	}
	n := width * height
	for len(c.order) > 0 && (len(c.order) >= 16 || c.bytes+n > 1<<20) {
		old := c.order[0]
		c.order = c.order[1:]
		c.bytes -= len(c.entries[old].Pix)
		delete(c.entries, old)
	}
	m := image.NewAlpha(image.Rect(0, 0, width, height))
	r := raster.NewRasterizer(width, height)
	if corner < 0 {
		start := traceArc(r, float64(width/2), float64(height/2), float64(width/2), float64(height/2), 0, -math.Pi*2, true)
		r.Add1(start)
	} else {
		cx, cy := []int{width, 0, 0, width}[corner], []int{height, height, 0, 0}[corner]
		r.Start(fixed.P(cx, cy))
		traceArc(r, float64(cx), float64(cy), float64(width), float64(height), math.Pi+float64(corner)*math.Pi/2, math.Pi/2, false)
		r.Add1(fixed.P(cx, cy))
	}
	p := raster.NewAlphaOverPainter(m)
	r.Rasterize(p)
	if c.entries == nil {
		c.entries = make(map[shapeMaskKey]*image.Alpha)
	}
	c.entries[k] = m
	c.order = append(c.order, k)
	c.bytes += n
	return m
}
