package ui

import (
	"sync"

	"github.com/allquixotic/fastrock/internal/desktop/font"
	"github.com/allquixotic/fastrock/internal/richtext"
	"golang.org/x/image/font/gofont/gobold"
	"golang.org/x/image/font/gofont/gobolditalic"
	"golang.org/x/image/font/gofont/gomonobold"
	"golang.org/x/image/font/gofont/gomonobolditalic"
	"golang.org/x/image/font/gofont/gomonoitalic"
)

type fontVariant uint8

const (
	regularFont fontVariant = iota
	boldFont
	italicFont
	boldItalicFont
	monoFont
	monoBoldFont
	monoItalicFont
	monoBoldItalicFont
)

var uiBold, uiBoldItalic = gobold.TTF, gobolditalic.TTF
var uiMonoBold, uiMonoItalic, uiMonoBoldItalic = gomonobold.TTF, gomonoitalic.TTF, gomonobolditalic.TTF
var sharedFontFaces sync.Map
var sharedFontSizes sync.Map

type fontKey struct {
	size    int
	variant fontVariant
}

func fontData(v fontVariant) []byte {
	switch v {
	case boldFont:
		return uiBold
	case italicFont:
		return uiItalic
	case boldItalicFont:
		return uiBoldItalic
	case monoFont:
		return uiMono
	case monoBoldFont:
		return uiMonoBold
	case monoItalicFont:
		return uiMonoItalic
	case monoBoldItalicFont:
		return uiMonoBoldItalic
	default:
		return uiRegular
	}
}

// Face access is synchronized by desktop. A finite size/variant key space
// shares glyph caches across drawing and background measurement.
func typeFace(size int, variant fontVariant) font.Face {
	key := fontKey{max(4, min(72, size)), variant}
	if value, ok := sharedFontFaces.Load(key); ok {
		return value.(font.Face)
	}
	face, err := font.NewFace(fontData(variant), key.size)
	if err != nil {
		panic(err)
	} // Embedded fallbacks were validated at startup.
	sharedFontSizes.Store(face, key.size)
	value, loaded := sharedFontFaces.LoadOrStore(key, face)
	if loaded {
		_ = face.Face.Close()
		sharedFontSizes.Delete(face)
	}
	result := value.(font.Face)
	return result
}

func fontPointSize(face font.Face) int {
	if size, ok := sharedFontSizes.Load(face); ok {
		return size.(int)
	}
	return max(4, face.Metrics().Height.Ceil())
}

func formattedFont(base int, f richtext.Format) (int, fontVariant) {
	variant := regularFont
	bold := f.Style&richtext.Bold != 0 || f.Heading > 0
	italic := f.Style&richtext.Italic != 0
	if bold {
		variant = boldFont
	}
	if italic {
		variant = italicFont
		if bold {
			variant = boldItalicFont
		}
	}
	if f.Style&richtext.Code != 0 {
		return max(4, base-1), variant + monoFont
	}
	if f.Heading == 1 {
		base += 7
	} else if f.Heading > 1 {
		base += 3
	}
	return base, variant
}

func drawFace(base int, f richtext.Format) font.Face {
	size, variant := formattedFont(base, f)
	return typeFace(size, variant)
}
