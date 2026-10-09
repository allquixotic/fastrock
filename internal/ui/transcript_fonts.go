package ui

import (
	"unicode"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/richtext"
	"golang.org/x/image/math/fixed"
)

// Wrapping uses the same selected faces as drawing, including mixed inline
// code/bold/italic and heading sizes. Positions remain document rune offsets.
func wrapTranscriptCell(doc *richtext.Document, start, end, width, size int) []transcriptLine {
	var rows []transcriptLine
	appendRow := func(from, to int) {
		row := transcriptLine{Height: nucular.FontHeight(typeFace(size, regularFont)) + 7}
		x := 4
		for i := from; i < to; {
			format, next := doc.RunAt(i)
			j := min(to, next)
			face := drawFace(size, format)
			run := transcriptRun{Text: string(doc.Text[i:j]), X: x, Format: format, Face: face}
			measureTranscriptRun(&run, face)
			run.Width = run.Advances[len(run.Advances)-1]
			row.Runs = append(row.Runs, run)
			row.Height = max(row.Height, nucular.FontHeight(face)+7)
			x += run.Width
			i = j
		}
		rows = append(rows, row)
	}
	rowStart, cursor, lastBreak := start, start, -1
	var x fixed.Int26_6
	for cursor < end {
		format, _ := doc.RunAt(cursor)
		face := drawFace(size, format)
		advance, _ := face.Face.GlyphAdvance(doc.Text[cursor])
		if cursor > rowStart {
			previous, _ := doc.RunAt(cursor - 1)
			if previous == format {
				advance += face.Face.Kern(doc.Text[cursor-1], doc.Text[cursor])
			} else {
				x = fixed.I(x.Ceil())
			}
		}
		if (x+advance).Ceil() > width && cursor > rowStart {
			cut := cursor
			if lastBreak > rowStart {
				cut = lastBreak
			}
			trim := cut
			for trim > rowStart && unicode.IsSpace(doc.Text[trim-1]) {
				trim--
			}
			appendRow(rowStart, trim)
			rowStart = cut
			for rowStart < end && unicode.IsSpace(doc.Text[rowStart]) {
				rowStart++
			}
			cursor, lastBreak, x = rowStart, -1, 0
			continue
		}
		x += advance
		cursor++
		if unicode.IsSpace(doc.Text[cursor-1]) {
			lastBreak = cursor
		}
	}
	if rowStart < end {
		appendRow(rowStart, end)
	}
	return rows
}
