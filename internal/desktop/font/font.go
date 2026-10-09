package font

import (
	"crypto/sha256"
	_ "embed"
	"image"
	"sync"

	"golang.org/x/image/font"
	"golang.org/x/image/math/fixed"

	"github.com/golang/freetype"
	"github.com/golang/freetype/truetype"
)

var fontsMu sync.Mutex
var fontsMap = map[[sha256.Size]byte]*truetype.Font{}

//go:embed DroidSansMono.ttf
var droidSansMonoFontData []byte

//go:embed DroidSansJapanese.ttf
var droidSansJapaneseData []byte

type Face struct {
	Face font.Face
}

// synchronizedFace protects both freetype's glyph cache and fallback lookup
// state when background layout and the renderer share a font face.
type synchronizedFace struct {
	mu   sync.Mutex
	face font.Face
}

func (f *synchronizedFace) Close() error {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.face.Close()
}
func (f *synchronizedFace) Glyph(dot fixed.Point26_6, r rune) (image.Rectangle, image.Image, image.Point, fixed.Int26_6, bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.face.Glyph(dot, r)
}
func (f *synchronizedFace) GlyphBounds(r rune) (fixed.Rectangle26_6, fixed.Int26_6, bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.face.GlyphBounds(r)
}
func (f *synchronizedFace) GlyphAdvance(r rune) (fixed.Int26_6, bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.face.GlyphAdvance(r)
}
func (f *synchronizedFace) Kern(a, b rune) fixed.Int26_6 {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.face.Kern(a, b)
}
func (f *synchronizedFace) Metrics() font.Metrics {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.face.Metrics()
}

func (face Face) MeasureString(text string) int {
	f := face.Face
	if safe, ok := f.(*synchronizedFace); ok {
		safe.mu.Lock()
		defer safe.mu.Unlock()
		f = safe.face
	}
	d := font.Drawer{Face: f}
	return d.MeasureString(text).Ceil()
}

func (face Face) Metrics() font.Metrics {
	return face.Face.Metrics()
}

// Returns default font (DroidSansMono) with specified size and scaling
func DefaultFont(size int, scaling float64) Face {
	face, err := NewFallbackFace(int(float64(size)*scaling), droidSansMonoFontData, droidSansJapaneseData)
	if err != nil {
		panic(err)
	}
	return face
}

// NewFace returns a new face by parsing the ttf font.
func NewFace(ttf []byte, size int) (Face, error) {
	_, face, err := newFaceIntl(ttf, size)
	return face, err
}

func newFaceIntl(ttf []byte, size int) (*truetype.Font, Face, error) {
	key := sha256.Sum256(ttf)
	fontsMu.Lock()
	defer fontsMu.Unlock()

	fnt, _ := fontsMap[key]
	if fnt == nil {
		var err error
		fnt, err = freetype.ParseFont(ttf)
		if err != nil {
			return nil, Face{}, err
		}
		fontsMap[key] = fnt
	}

	return fnt, Face{&synchronizedFace{face: truetype.NewFace(fnt, &truetype.Options{Size: float64(size), Hinting: font.HintingFull, DPI: 72})}}, nil
}

func NewFallbackFace(size int, ttfs ...[]byte) (Face, error) {
	fnts := make([]*truetype.Font, len(ttfs))
	faces := make([]Face, len(ttfs))
	for i := range ttfs {
		var err error
		fnts[i], faces[i], err = newFaceIntl(ttfs[i], size)
		if err != nil {
			return Face{}, err
		}
	}
	return Face{&synchronizedFace{face: &fallbackFace{fnts: fnts, faces: faces, runeToFace: make(map[rune]int)}}}, nil
}

type fallbackFace struct {
	fnts       []*truetype.Font
	faces      []Face
	runeToFace map[rune]int
}

func (ff *fallbackFace) Close() error {
	var err error
	for i := range ff.faces {
		e1 := ff.faces[i].Face.Close()
		if err == nil {
			err = e1
		}
	}
	return err
}

func (ff *fallbackFace) Glyph(dot fixed.Point26_6, r rune) (image.Rectangle, image.Image, image.Point, fixed.Int26_6, bool) {
	i := ff.lookupRune(r)
	return ff.faces[i].Face.Glyph(dot, r)
}

func (ff *fallbackFace) GlyphAdvance(r rune) (fixed.Int26_6, bool) {
	i := ff.lookupRune(r)
	return ff.faces[i].Face.GlyphAdvance(r)
}

func (ff *fallbackFace) GlyphBounds(r rune) (fixed.Rectangle26_6, fixed.Int26_6, bool) {
	i := ff.lookupRune(r)
	return ff.faces[i].Face.GlyphBounds(r)
}

func (ff *fallbackFace) lookupRune(r rune) int {
	if i, ok := ff.runeToFace[r]; ok {
		return i
	}
	for i := range ff.fnts {
		if ff.fnts[i].Index(r) != 0 {
			ff.runeToFace[r] = i
			return i
		}
	}
	ff.runeToFace[r] = 0
	return 0
}

func (ff *fallbackFace) Kern(r1, r2 rune) fixed.Int26_6 {
	i1 := ff.lookupRune(r1)
	i2 := ff.lookupRune(r2)
	if i1 == i2 {
		return ff.faces[i1].Face.Kern(r1, r2)
	}
	return 0
}

func (ff *fallbackFace) Metrics() font.Metrics {
	return ff.faces[0].Face.Metrics()
}
