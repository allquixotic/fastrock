package richtext

import (
	"bytes"
	"encoding/json"
	"fmt"
	"slices"
	"unicode/utf8"
)

// Span ends at a Unicode code-point offset, exclusive. Adjacent spans cover
// the whole saved text; one format serves an arbitrarily long text run.
type Span struct {
	End    int
	Format Format
}

// Saved is an immutable draft checkpoint. Text uses UTF-8 and formatting uses
// runs instead of one JSON object per rune. Undo history stays local.
type Saved struct {
	Original string
	Text     string `json:",omitempty"`
	Spans    []Span `json:",omitempty"`
	Changed  bool
}

func spansFromMarks(marks []Format) []Span {
	var spans []Span
	for i, format := range marks {
		if len(spans) != 0 && spans[len(spans)-1].Format == format {
			spans[len(spans)-1].End = i + 1
		} else {
			spans = append(spans, Span{End: i + 1, Format: format})
		}
	}
	return spans
}

func (s Saved) valid() bool {
	end := 0
	length := utf8.RuneCountInString(s.Text)
	for _, span := range s.Spans {
		if span.End <= end || span.End > length {
			return false
		}
		end = span.End
	}
	return end == length
}

// Accept the original numeric rune array/per-rune Marks representation when
// migrating existing sessions, but only write the compact representation.
func (s *Saved) UnmarshalJSON(data []byte) error {
	var wire struct {
		Original string
		Text     json.RawMessage
		Spans    []Span
		Marks    []Format
		Changed  bool
	}
	if err := json.Unmarshal(data, &wire); err != nil {
		return err
	}
	next := Saved{Original: wire.Original, Spans: wire.Spans, Changed: wire.Changed}
	raw := bytes.TrimSpace(wire.Text)
	if len(raw) > 0 && raw[0] == '[' {
		var text []rune
		if err := json.Unmarshal(raw, &text); err != nil {
			return err
		}
		if len(text) != len(wire.Marks) {
			return fmt.Errorf("rich draft text/format lengths differ")
		}
		next.Text, next.Spans = string(text), spansFromMarks(wire.Marks)
	} else if len(raw) > 0 && !bytes.Equal(raw, []byte("null")) {
		if err := json.Unmarshal(raw, &next.Text); err != nil {
			return err
		}
	}
	if next.Changed && !next.valid() {
		return fmt.Errorf("invalid rich draft format spans")
	}
	*s = next
	return nil
}

func (d *Document) Save() Saved {
	s := Saved{Original: d.original, Changed: d.changed}
	if d.changed {
		s.Text = string(d.Text)
		s.Spans = slices.Clone(d.spans)
	}
	return s
}

func Restore(s Saved) *Document {
	d := Parse(s.Original)
	if s.Changed && s.valid() {
		d.captureOriginal()
		d.Text = []rune(s.Text)
		d.spans = nil
		for _, span := range s.Spans {
			format := span.Format
			format.Link = SafeLink(format.Link)
			d.spans = appendSpan(d.spans, span.End, format)
		}
		d.invalidate()
	}
	return d
}
