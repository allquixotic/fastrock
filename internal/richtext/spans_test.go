package richtext

import (
	"math/rand"
	"reflect"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"
)

func denseMarks(d *Document) []Format {
	marks := make([]Format, len(d.Text))
	start := 0
	for _, span := range d.spans {
		for i := start; i < span.End; i++ {
			marks[i] = span.Format
		}
		start = span.End
	}
	return marks
}

func assertSpans(t *testing.T, d *Document) {
	t.Helper()
	end := 0
	for i, s := range d.spans {
		if s.End <= end || s.End > len(d.Text) || i > 0 && d.spans[i-1].Format == s.Format {
			t.Fatalf("invalid/noncanonical spans: %#v for %q", d.spans, string(d.Text))
		}
		end = s.End
	}
	if end != len(d.Text) {
		t.Fatalf("spans end at %d; text ends at %d", end, len(d.Text))
	}
}

func TestV7SpanEditsMatchDenseOracle(t *testing.T) {
	checkSpanEdits(t, 7)
}

func FuzzRichEditHistory(f *testing.F) {
	f.Add(int64(7))
	f.Add(int64(1))
	f.Fuzz(func(t *testing.T, seed int64) { checkSpanEdits(t, seed) })
}

func checkSpanEdits(t *testing.T, seed int64) {
	d := Parse("<p><b>AB界</b> <i>🙂café</i></p>")
	d.history = &historyPool{limit: 8 << 20}
	text, marks := slices.Clone(d.Text), denseMarks(d)
	rng := rand.New(rand.NewSource(seed))
	type state struct {
		text  []rune
		marks []Format
	}
	states := []state{{slices.Clone(text), slices.Clone(marks)}}
	for range 400 {
		start := rng.Intn(len(text) + 1)
		end := start + rng.Intn(len(text)-start+1)
		d.lastEdit = time.Time{}
		switch rng.Intn(5) {
		case 0:
			inserted := []rune([]string{"", "é", "界🙂", "ab\n"}[rng.Intn(4)])
			f := Format{}
			if len(marks) > 0 {
				f = marks[max(0, start-1)]
			}
			replacement := make([]Format, len(inserted))
			for i := range replacement {
				replacement[i] = f
			}
			if d.ApplyEdit(start, end-start, inserted) {
				text = slices.Replace(text, start, end, inserted...)
				marks = slices.Replace(marks, start, end, replacement...)
			}
		case 1:
			if start == end {
				continue
			}
			style := Style(1 << rng.Intn(5))
			remove := true
			for _, f := range marks[start:end] {
				if f.Style&style == 0 {
					remove = false
				}
			}
			d.Toggle(start, end, style)
			for i := start; i < end; i++ {
				if remove {
					marks[i].Style &^= style
				} else {
					marks[i].Style |= style
				}
			}
		case 2:
			if start == end {
				continue
			}
			link := []string{"", "https://example.com", "mailto:a@example.com"}[rng.Intn(3)]
			d.Link(start, end, link)
			for i := start; i < end; i++ {
				marks[i].Link = link
			}
		case 3:
			heading, list, quote := uint8(rng.Intn(4)), uint8(rng.Intn(3)), rng.Intn(2) == 1
			d.Paragraph(start, end, heading, list, quote)
			for start > 0 && text[start-1] != '\n' {
				start--
			}
			for end < len(text) && text[end] != '\n' {
				end++
			}
			for i := start; i < end; i++ {
				marks[i].Heading, marks[i].List, marks[i].Quote = heading, list, quote
			}
		case 4:
			f := Format{}
			if len(marks) > 0 {
				f = marks[min(start, len(marks)-1)]
			}
			f.Style ^= Italic
			d.Toggle(start, start, Italic)
			d.ApplyEdit(start, 0, []rune("🚀"))
			d.ClearPending()
			text = slices.Insert(text, start, '🚀')
			marks = slices.Insert(marks, start, f)
		}
		assertSpans(t, d)
		if string(d.Text) != string(text) || !reflect.DeepEqual(denseMarks(d), marks) {
			t.Fatalf("span/oracle mismatch: %q", string(d.Text))
		}
		prior := states[len(states)-1]
		if string(text) != string(prior.text) || !reflect.DeepEqual(marks, prior.marks) {
			states = append(states, state{slices.Clone(text), slices.Clone(marks)})
		}
	}
	// The bounded history keeps the latest 100 independent operations. Each
	// undo and redo must reconstruct exactly the dense reference state.
	count := len(d.undo)
	for i := 1; i <= count; i++ {
		if !d.Undo(false) {
			t.Fatal("missing undo")
		}
		want := states[len(states)-1-i]
		assertSpans(t, d)
		if string(d.Text) != string(want.text) || !reflect.DeepEqual(denseMarks(d), want.marks) {
			t.Fatal("undo differs from oracle", i)
		}
	}
	for i := count; i > 0; i-- {
		if !d.Undo(true) {
			t.Fatal("missing redo")
		}
		want := states[len(states)-i]
		assertSpans(t, d)
		if string(d.Text) != string(want.text) || !reflect.DeepEqual(denseMarks(d), want.marks) {
			t.Fatal("redo differs from oracle", i)
		}
	}
}

func TestV7RichHistorySharedBudgetAndDeltaSize(t *testing.T) {
	pool := &historyPool{limit: 16 << 10}
	var documents []*Document
	for range 12 {
		d := Parse("<p>" + strings.Repeat("x", 100000) + "</p>")
		d.history = pool
		for range 8 {
			d.lastEdit = time.Time{}
			d.ApplyEdit(50000, 0, []rune("界"))
		}
		if len(d.spans) != 1 || pool.bytes > pool.limit {
			t.Fatal("unbounded formatting/history", len(d.spans), pool.bytes)
		}
		documents = append(documents, d)
	}
	for _, d := range documents {
		if len(d.Text) != 100008 {
			t.Fatal("budget pressure discarded unsaved content")
		}
		for _, group := range d.undo {
			if len(group.edits) != 1 || len(group.edits[0].beforeText) != 0 || len(group.edits[0].afterText) != 1 {
				t.Fatal("typing stored whole document instead of delta")
			}
		}
		d.DiscardHistory()
	}
	if pool.bytes != 0 || pool.groups.Len() != 0 {
		t.Fatal("history not released", pool.bytes)
	}
}

func TestV7FormattingReleasesOversizedSpanBuffers(t *testing.T) {
	d := Parse("<p>" + strings.Repeat("a<b>b</b>", 2000) + "</p>")
	d.history = &historyPool{limit: 1}
	if len(d.spans) < 2000 {
		t.Fatal("fixture did not create enough spans")
	}
	d.Toggle(0, len(d.Text), Bold)
	if len(d.spans) != 1 || cap(d.spans) > 64 || cap(d.spareSpans) > 64 {
		t.Fatal("coalesced formatting retained old capacity", len(d.spans), cap(d.spans), cap(d.spareSpans))
	}
	if len(d.Text) != 4000 {
		t.Fatal("formatting discarded text")
	}
}

func TestV7HistoryBudgetKeepsRedoChain(t *testing.T) {
	pool := &historyPool{limit: 10000}
	d := Parse("<p>base</p>")
	d.history = pool
	for range 12 {
		d.lastEdit = time.Time{}
		d.ApplyEdit(len(d.Text), 0, []rune("x"))
	}
	for range 12 {
		if !d.Undo(false) {
			t.Fatal("missing undo")
		}
	}
	other := Parse("")
	other.history = pool
	for range 25 {
		other.lastEdit = time.Time{}
		other.ApplyEdit(len(other.Text), 0, []rune("y"))
	}
	for n := 1; d.Undo(true); n++ {
		if string(d.Text) != "base"+strings.Repeat("x", n) {
			t.Fatal("redo skipped an evicted intermediate state")
		}
	}
	if pool.bytes > pool.limit {
		t.Fatal("budget exceeded")
	}
}

func TestV7SharedHistoryConcurrentEditors(t *testing.T) {
	pool := &historyPool{limit: 8192}
	var wg sync.WaitGroup
	for range 8 {
		wg.Go(func() {
			d := Parse("<p>original</p>")
			d.history = pool
			for i := range 200 {
				d.ApplyEdit(len(d.Text), 0, []rune("x"))
				if i%7 == 0 {
					d.Undo(false)
					d.Undo(true)
				}
			}
			d.DiscardHistory()
		})
	}
	wg.Wait()
	if pool.bytes != 0 || pool.groups.Len() != 0 {
		t.Fatal("concurrent history leaked", pool.bytes)
	}
}

func TestV9OriginalHTMLSurvivesHistoryEviction(t *testing.T) {
	const source = "<DIV><B>original</B></DIV>\n"
	d := Parse(source)
	d.history = &historyPool{limit: 1}
	d.ApplyEdit(len(d.Text), 0, []rune("changed"))
	d.ApplyEdit(len(d.Text)-7, 7, nil)
	if d.Undo(false) {
		t.Fatal("fixture did not evict history")
	}
	if d.HTML() != source {
		t.Fatal("history eviction lost exact unchanged HTML", d.HTML())
	}
	d.Toggle(0, len(d.Text), Italic)
	d.Toggle(0, len(d.Text), Italic)
	if d.HTML() != source {
		t.Fatal("formatting reversal lost exact unchanged HTML", d.HTML())
	}
}

func BenchmarkV7EditLargeDocument(b *testing.B) {
	d := Parse("<p>" + strings.Repeat("x", 500000) + "</p>")
	d.history = &historyPool{limit: 8 << 20}
	d.ApplyEdit(250000, 0, []rune("a"))
	d.ApplyEdit(250000, 1, nil)
	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		d.ApplyEdit(250000, 0, []rune("a"))
		d.ApplyEdit(250000, 1, nil)
	}
}
