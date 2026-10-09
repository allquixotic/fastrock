package nucular

import (
	"strings"
	"sync"

	"github.com/aarzilli/nucular/command"
	"github.com/aarzilli/nucular/font"
	"github.com/aarzilli/nucular/rect"

	"golang.org/x/image/math/fixed"
)

// A bounded typed FIFO avoids interface boxing and linked-list nodes on the hot
// measurement path. Cache misses measure outside the shared cache lock.
var widthMu sync.RWMutex
var widthValues = make(map[fontWidthCacheKey]int, 2048)
var widthKeys = make([]fontWidthCacheKey, 2048)
var widthNext int

func ChangeFontWidthCache(size int) {
	widthMu.Lock()
	defer widthMu.Unlock()
	if size > len(widthKeys) {
		widthValues = make(map[fontWidthCacheKey]int, size)
		widthKeys = make([]fontWidthCacheKey, size)
		widthNext = 0
	}
}

type fontWidthCacheKey struct {
	f      font.Face
	string string
}

func FontWidth(f font.Face, str string) int {
	maxw := 0
	for {
		newline := strings.Index(str, "\n")
		line := str
		if newline >= 0 {
			line = str[:newline]
		}

		k := fontWidthCacheKey{f, line}

		widthMu.RLock()
		w, ok := widthValues[k]
		widthMu.RUnlock()
		if !ok {
			w = f.MeasureString(line)
			// Do not retain arbitrarily large pasted strings in the global cache.
			if len(line) <= 4096 {
				widthMu.Lock()
				if _, exists := widthValues[k]; !exists {
					delete(widthValues, widthKeys[widthNext])
					k.string = strings.Clone(line)
					widthValues[k] = w
					widthKeys[widthNext] = k
					widthNext = (widthNext + 1) % len(widthKeys)
				}
				widthMu.Unlock()
			}
		}

		if w > maxw {
			maxw = w
		}

		if newline >= 0 {
			str = str[newline+1:]
		} else {
			break
		}
	}
	return maxw
}

func glyphAdvance(f font.Face, ch rune) int {
	a, _ := f.Face.GlyphAdvance(ch)
	return a.Ceil()
}

func measureRunes(f font.Face, runes []rune) int {
	var advance fixed.Int26_6
	prevC := rune(-1)
	fc := f.Face
	for _, c := range runes {
		if prevC >= 0 {
			advance += fc.Kern(prevC, c)
		}
		a, ok := fc.GlyphAdvance(c)
		if !ok {
			// TODO: is falling back on the U+FFFD glyph the responsibility of
			// the Drawer or the Face?
			// TODO: set prevC = '\ufffd'?
			continue
		}
		advance += a
		prevC = c
	}
	return advance.Ceil()
}

func widgetTextWrap(o *command.Buffer, b rect.Rect, str []rune, t *textWidget, f font.Face) {
	height := FontHeight(f)
	line := rect.Rect{X: b.X + t.Padding.X, Y: b.Y + t.Padding.Y, W: max(1, b.W-2*t.Padding.X), H: height}
	text := textWidget{Text: t.Text, Background: t.Background}
	for _, value := range WrapText(f, string(str), line.W) {
		if line.Y+height > b.Y+b.H-t.Padding.Y {
			break
		}
		widgetText(o, line, value, &text, "LC", f)
		line.Y += height
	}
}

func textClamp(f font.Face, text []rune, space int) []rune {
	text_width := 0
	fc := f.Face
	for i, ch := range text {
		xwfixed, _ := fc.GlyphAdvance(ch)
		xw := xwfixed.Ceil()
		if text_width+xw >= space {
			return text[:i]
		}
		text_width += xw
	}
	return text
}
