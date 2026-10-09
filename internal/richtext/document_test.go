package richtext

import (
	"strings"
	"testing"
)

func TestRallyHTMLRoundTripAndUnicodeEditing(t *testing.T) {
	source := `<h2>Acceptance</h2><p><strong>Ship 🚀</strong> with <em>care</em> &amp; <a href="https://example.com">review</a>.</p><ul><li>One</li><li>Two</li></ul>`
	d := Parse(source)
	if d.HTML() != source {
		t.Fatal("opening an item rewrote its HTML")
	}
	if got := string(d.Text); got != "Acceptance\nShip 🚀 with care & review.\nOne\nTwo" {
		t.Fatalf("plain text %q", got)
	}
	d.Sync([]rune(strings.Replace(string(d.Text), "Ship 🚀", "Ship now 🚀", 1)))
	if !strings.Contains(d.HTML(), "<strong>Ship now 🚀</strong>") {
		t.Fatal(d.HTML())
	}
	if !strings.Contains(d.HTML(), `<a href="https://example.com">review</a>`) {
		t.Fatal(d.HTML())
	}
	if !strings.Contains(d.HTML(), "<ul><li>One</li><li>Two</li></ul>") {
		t.Fatal(d.HTML())
	}
	if !d.Undo(false) || d.HTML() != source {
		t.Fatalf("undo lost original markup: %s", d.HTML())
	}
	if !d.Undo(true) || !strings.Contains(d.HTML(), "now") {
		t.Fatal("redo failed")
	}
}

func TestSelectionFormattingAndReplacement(t *testing.T) {
	d := Parse("<p>hello world</p>")
	d.Toggle(0, 5, Bold)
	d.Toggle(0, 5, Italic)
	d.Link(0, 5, "https://example.com")
	if d.HTML() != `<p><a href="https://example.com"><strong><em>hello</em></strong></a> world</p>` {
		t.Fatal(d.HTML())
	}
	d.Sync([]rune("hello brave world"))
	if len(d.Text) != len(d.Marks) {
		t.Fatal("format offsets lost")
	}
	d.Paragraph(0, 3, 2, 0, false)
	if !strings.HasPrefix(d.HTML(), "<h2>") {
		t.Fatal(d.HTML())
	}
	d.Toggle(0, 5, Bold)
	if strings.Contains(d.HTML(), "<strong>") {
		t.Fatal("toggle off failed")
	}
}

func TestUnsafeHTMLIsNotRenderedAsContent(t *testing.T) {
	d := Parse(`<p>Safe<script>alert(1)</script><img src="javascript:alert(1)" alt="picture"><a href="javascript:alert(1)">link</a></p>`)
	if string(d.Text) != "Safe[picture]link" {
		t.Fatal(string(d.Text))
	}
	for _, f := range d.Marks {
		if f.Link != "" {
			t.Fatal("unsafe link accepted")
		}
	}
	for _, s := range []string{"javascript:alert(1)", "data:text/html,x", "file:///secret"} {
		if SafeLink(s) != "" {
			t.Fatal(s)
		}
	}
	d.Sync(append(d.Text, '!'))
	if strings.Contains(d.HTML(), "script") || strings.Contains(d.HTML(), "src=") {
		t.Fatal(d.HTML())
	}
}

func TestUnchangedDocumentAllocations(t *testing.T) {
	d := Parse("<p><b>Keep this text</b></p>")
	if got := testing.AllocsPerRun(100, func() { d.Sync(d.Text); _ = d.HTML() }); got != 0 {
		t.Fatalf("%g allocations on unchanged content", got)
	}
}

func TestRepeatedEditsReuseMarksAndKeepSurroundingStyles(t *testing.T) {
	d := Parse("<p><b>before</b> <i>after</i></p>")
	before := []rune("before after")
	inserted := []rune("before inserted after")
	d.Sync(inserted)
	d.Sync(before)
	if n := testing.AllocsPerRun(100, func() { d.Sync(inserted); d.Sync(before) }); n != 0 {
		t.Fatalf("warm coalesced edits allocate %g", n)
	}
	if got := d.HTML(); got != "<p><b>before</b> <i>after</i></p>" {
		t.Fatal(got)
	}
}
