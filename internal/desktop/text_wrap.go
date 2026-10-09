package desktop

import (
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"strings"
	"unicode"
)

// WrapText measures the same lines as LabelWrap, including explicit newlines.
// It lets callers reserve enough space without guessing average glyph widths.
func WrapText(face font.Face, text string, width int) []string {
	var lines []string
	for _, paragraph := range strings.Split(text, "\n") {
		runes := []rune(paragraph)
		if len(runes) == 0 {
			lines = append(lines, "")
			continue
		}
		for len(runes) > 0 {
			n := len(textClamp(face, runes, max(1, width)))
			if n == 0 {
				n = 1
			}
			if n < len(runes) {
				for j := n - 1; j > 0; j-- {
					if unicode.IsSpace(runes[j]) {
						n = j
						break
					}
				}
			}
			lines = append(lines, string(runes[:n]))
			runes = runes[n:]
			for len(runes) > 0 && unicode.IsSpace(runes[0]) {
				runes = runes[1:]
			}
		}
	}
	return lines
}
