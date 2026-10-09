package richtext

import (
	"crypto/sha256"
	"encoding/binary"
	"slices"
	"sort"
)

// RunAt returns the actual format and exclusive end of the run containing i.
// Unlike FormatAt, it does not include a pending insertion-only format.
func (d *Document) RunAt(i int) (Format, int) {
	if len(d.spans) == 0 {
		return Format{}, 0
	}
	i = max(0, min(i, len(d.Text)-1))
	n := sort.Search(len(d.spans), func(n int) bool { return d.spans[n].End > i })
	s := d.spans[n]
	return s.Format, s.End
}

func appendSpan(spans []Span, end int, format Format) []Span {
	if end <= 0 {
		return spans
	}
	if n := len(spans); n > 0 && spans[n-1].Format == format {
		spans[n-1].End = end
		return spans
	}
	return append(spans, Span{End: end, Format: format})
}

func (d *Document) appendRune(r rune, format Format) {
	d.Text = append(d.Text, r)
	d.spans = appendSpan(d.spans, len(d.Text), format)
}

func (d *Document) truncate(length int) {
	d.Text = d.Text[:length]
	if length == 0 {
		clear(d.spans)
		d.spans = d.spans[:0]
		return
	}
	i := sort.Search(len(d.spans), func(i int) bool { return d.spans[i].End >= length })
	clear(d.spans[i+1:])
	d.spans = d.spans[:i+1]
	d.spans[i].End = length
}

// sliceSpans owns its result; all offsets are relative to start.
func (d *Document) sliceSpans(start, end int) []Span {
	var spans []Span
	for start < end {
		format, next := d.RunAt(start)
		base := 0
		if len(spans) != 0 {
			base = spans[len(spans)-1].End
		}
		next = min(end, next)
		spans = appendSpan(spans, base+next-start, format)
		start = next
	}
	return spans
}

// Replace an interval and shift only span endpoints. Two scratch slices avoid
// reallocating all formatting storage during ordinary typing.
func (d *Document) replaceSpans(start, removed, inserted int, spans []Span) {
	next := d.spareSpans[:0]
	for _, s := range d.spans {
		end := min(start, s.End)
		if end > 0 {
			next = appendSpan(next, end, s.Format)
		}
		if s.End >= start {
			break
		}
	}
	for _, s := range spans {
		next = appendSpan(next, start+s.End, s.Format)
	}
	oldEnd := start + removed
	for _, s := range d.spans {
		if s.End > oldEnd {
			next = appendSpan(next, s.End-removed+inserted, s.Format)
		}
	}
	old := d.spans
	if cap(next) > max(64, 4*len(next)) {
		next = slices.Clone(next)
	}
	d.spans = next
	clear(old)
	if cap(old) > max(64, 4*len(next)) {
		d.spareSpans = nil
	} else {
		d.spareSpans = old[:0]
	}
}

// Keep a fixed-size fingerprint of the original editable representation instead
// of a pinned full-document undo snapshot. Exact source HTML remains available.
func (d *Document) fingerprint() [sha256.Size]byte {
	h := sha256.New()
	var buf [16]byte
	binary.LittleEndian.PutUint64(buf[:8], uint64(len(d.Text)))
	binary.LittleEndian.PutUint64(buf[8:], uint64(len(d.spans)))
	h.Write(buf[:])
	h.Write([]byte(string(d.Text)))
	for _, s := range d.spans {
		binary.LittleEndian.PutUint64(buf[:8], uint64(s.End))
		binary.LittleEndian.PutUint32(buf[8:12], uint32(len(s.Format.Link)))
		buf[12], buf[13], buf[14], buf[15] = byte(s.Format.Style), s.Format.Heading, s.Format.List, 0
		if s.Format.Quote {
			buf[15] = 1
		}
		h.Write(buf[:])
		h.Write([]byte(s.Format.Link))
	}
	var out [sha256.Size]byte
	copy(out[:], h.Sum(nil))
	return out
}

func (d *Document) captureOriginal() {
	if !d.changed {
		d.originalFingerprint = d.fingerprint()
	}
}
