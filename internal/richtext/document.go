// Package richtext converts Rally HTML to an editable, styled Unicode document.
// It never executes HTML or fetches resources. Unedited HTML is returned verbatim.
package richtext

import (
	"fmt"
	"html"
	"net/url"
	"slices"
	"strings"
	"time"
	"unicode"

	xhtml "golang.org/x/net/html"
	"golang.org/x/net/html/atom"
)

type Style uint8

const (
	Bold Style = 1 << iota
	Italic
	Underline
	Strike
	Code
)

type Format struct {
	Style   Style
	Link    string
	Heading uint8
	List    uint8 // 1: unordered, 2: ordered
	Quote   bool
}

type snapshot struct {
	Text  []rune
	Marks []Format
}

type Document struct {
	Revision   uint64
	Text       []rune
	Marks      []Format // one entry per Unicode code point, including paragraph separators
	original   string
	initial    *snapshot
	changed    bool
	pending    *Format
	undo, redo []snapshot
	html       string
	dirty      bool
	lastEdit   time.Time
}

func Parse(source string) *Document {
	d := &Document{original: source}
	nodes, err := xhtml.ParseFragment(strings.NewReader(source), &xhtml.Node{Type: xhtml.ElementNode, Data: "div", DataAtom: atom.Div})
	if err != nil {
		d.Text = []rune(source)
		d.Marks = make([]Format, len(d.Text))
		return d
	}
	var appendText func(string, Format, bool)
	appendText = func(s string, f Format, pre bool) {
		for _, r := range s {
			if !pre && unicode.IsSpace(r) {
				if len(d.Text) == 0 || unicode.IsSpace(d.Text[len(d.Text)-1]) {
					continue
				}
				r = ' '
			}
			d.Text = append(d.Text, r)
			d.Marks = append(d.Marks, f)
		}
	}
	newline := func(f Format) {
		for len(d.Text) > 0 && d.Text[len(d.Text)-1] == ' ' {
			d.Text = d.Text[:len(d.Text)-1]
			d.Marks = d.Marks[:len(d.Marks)-1]
		}
		if len(d.Text) > 0 && d.Text[len(d.Text)-1] != '\n' {
			d.Text = append(d.Text, '\n')
			d.Marks = append(d.Marks, f)
		}
	}
	var walk func(*xhtml.Node, Format, bool)
	walk = func(n *xhtml.Node, f Format, pre bool) {
		if n.Type == xhtml.TextNode {
			appendText(n.Data, f, pre)
			return
		}
		if n.Type != xhtml.ElementNode {
			return
		}
		tag := n.Data
		switch tag {
		case "script", "style", "iframe", "object", "embed", "svg", "math":
			return
		}
		block := false
		switch tag {
		case "p", "div", "section", "article", "pre", "li", "blockquote", "tr":
			block = true
		case "h1", "h2", "h3", "h4", "h5", "h6":
			block = true
			f.Heading = uint8(tag[1] - '0')
			f.Style |= Bold
		}
		if block {
			newline(f)
		}
		switch tag {
		case "b", "strong", "th":
			f.Style |= Bold
		case "i", "em":
			f.Style |= Italic
		case "u":
			f.Style |= Underline
		case "s", "del", "strike":
			f.Style |= Strike
		case "code", "pre":
			f.Style |= Code
		case "blockquote":
			f.Quote = true
		case "ul":
			f.List = 1
		case "ol":
			f.List = 2
		case "a":
			for _, a := range n.Attr {
				if a.Key == "href" {
					f.Link = SafeLink(a.Val)
				}
			}
		case "br":
			d.Text = append(d.Text, '\n')
			d.Marks = append(d.Marks, f)
			return
		case "img":
			alt := "image"
			for _, a := range n.Attr {
				if a.Key == "alt" && a.Val != "" {
					alt = a.Val
				}
			}
			appendText("["+alt+"]", f, false)
			return
		}
		for _, a := range n.Attr {
			if a.Key != "style" {
				continue
			}
			for _, decl := range strings.Split(strings.ToLower(a.Val), ";") {
				parts := strings.SplitN(decl, ":", 2)
				if len(parts) != 2 {
					continue
				}
				key, value := strings.TrimSpace(parts[0]), strings.TrimSpace(parts[1])
				if key == "font-weight" && (value == "bold" || value == "700" || value == "600") {
					f.Style |= Bold
				}
				if key == "font-style" && value == "italic" {
					f.Style |= Italic
				}
				if key == "text-decoration" && strings.Contains(value, "underline") {
					f.Style |= Underline
				}
			}
		}
		for c := n.FirstChild; c != nil; c = c.NextSibling {
			walk(c, f, pre || tag == "pre")
		}
		if tag == "td" || tag == "th" {
			appendText("\t", f, true)
		}
		if block {
			newline(f)
		}
	}
	for _, n := range nodes {
		walk(n, Format{}, false)
	}
	for len(d.Text) > 0 && unicode.IsSpace(d.Text[len(d.Text)-1]) {
		d.Text = d.Text[:len(d.Text)-1]
		d.Marks = d.Marks[:len(d.Marks)-1]
	}
	return d
}

func SafeLink(s string) string {
	u, e := url.Parse(strings.TrimSpace(s))
	if e != nil {
		return ""
	}
	switch strings.ToLower(u.Scheme) {
	case "https", "http", "mailto":
		return u.String()
	}
	return ""
}

func (d *Document) record() {
	d.lastEdit = time.Time{}
	d.undo = append(d.undo, snapshot{slices.Clone(d.Text), slices.Clone(d.Marks)})
	if d.initial == nil {
		s := d.undo[len(d.undo)-1]
		d.initial = &s
	}
	if len(d.undo) > 100 {
		copy(d.undo, d.undo[len(d.undo)-100:])
		clear(d.undo[100:])
		d.undo = d.undo[:100]
	}
	d.redo = nil
	d.trimHistory()
}

// Coalesce typing bursts and bound history memory even for long descriptions.
func (d *Document) recordEdit() {
	now := time.Now()
	if d.lastEdit.IsZero() || now.Sub(d.lastEdit) > 650*time.Millisecond {
		d.record()
	}
	d.lastEdit = now
}
func (d *Document) trimHistory() {
	const budget = 8 << 20
	cost := func(s snapshot) int { return len(s.Text)*4 + len(s.Marks)*40 }
	total := 0
	for _, s := range d.undo {
		total += cost(s)
	}
	for _, s := range d.redo {
		total += cost(s)
	}
	for total > budget && len(d.undo) > 0 {
		total -= cost(d.undo[0])
		d.undo[0] = snapshot{}
		d.undo = d.undo[1:]
	}
	for total > budget && len(d.redo) > 0 {
		total -= cost(d.redo[0])
		d.redo[0] = snapshot{}
		d.redo = d.redo[1:]
	}
}
func (d *Document) invalidate() { d.changed = true; d.dirty = true; d.Revision++ }

// Sync reconciles a native editor insertion/deletion while retaining surrounding styles.
// Unchanged frames do no allocation.
func (d *Document) Sync(next []rune) bool {
	if slices.Equal(d.Text, next) {
		return false
	}
	d.recordEdit()
	prefix := 0
	for prefix < min(len(d.Text), len(next)) && d.Text[prefix] == next[prefix] {
		prefix++
	}
	suffix := 0
	for suffix < min(len(d.Text)-prefix, len(next)-prefix) && d.Text[len(d.Text)-1-suffix] == next[len(next)-1-suffix] {
		suffix++
	}
	f := Format{}
	if prefix > 0 {
		f = d.Marks[prefix-1]
	} else if len(d.Marks) > 0 {
		f = d.Marks[0]
	}
	if d.pending != nil {
		f = *d.pending
	}
	marks := slices.Grow(d.Marks, max(0, len(next)-len(d.Marks)))[:len(next)]
	// Move the preserved tail first: insertion and deletion can overlap the
	// existing storage. Reuse its capacity across keystrokes.
	copy(marks[len(next)-suffix:], d.Marks[len(d.Marks)-suffix:])
	for i := prefix; i < len(next)-suffix; i++ {
		marks[i] = f
	}
	d.Text = append(d.Text[:0], next...)
	d.Marks = marks
	d.invalidate()
	return true
}

func (d *Document) Selection(start, end int) (int, int) {
	if start > end {
		start, end = end, start
	}
	return max(0, min(start, len(d.Text))), max(0, min(end, len(d.Text)))
}

func (d *Document) FormatAt(i int) Format {
	if d.pending != nil {
		return *d.pending
	}
	if len(d.Marks) == 0 {
		return Format{}
	}
	return d.Marks[max(0, min(i, len(d.Marks)-1))]
}

func (d *Document) ClearPending() { d.pending = nil }

func (d *Document) Toggle(start, end int, style Style) {
	start, end = d.Selection(start, end)
	if start == end {
		f := d.FormatAt(start)
		f.Style ^= style
		d.pending = &f
		return
	}
	d.record()
	remove := true
	for _, f := range d.Marks[start:end] {
		if f.Style&style == 0 {
			remove = false
			break
		}
	}
	for i := start; i < end; i++ {
		if remove {
			d.Marks[i].Style &^= style
		} else {
			d.Marks[i].Style |= style
		}
	}
	d.invalidate()
}

func (d *Document) Link(start, end int, link string) {
	start, end = d.Selection(start, end)
	if start == end {
		return
	}
	d.record()
	link = SafeLink(link)
	for i := start; i < end; i++ {
		d.Marks[i].Link = link
	}
	d.invalidate()
}

func (d *Document) Paragraph(start, end int, heading, list uint8, quote bool) {
	start, end = d.Selection(start, end)
	for start > 0 && d.Text[start-1] != '\n' {
		start--
	}
	for end < len(d.Text) && d.Text[end] != '\n' {
		end++
	}
	d.record()
	for i := start; i < end; i++ {
		d.Marks[i].Heading = heading
		d.Marks[i].List = list
		d.Marks[i].Quote = quote
	}
	d.invalidate()
}

func (d *Document) Undo(redo bool) bool {
	d.lastEdit = time.Time{}
	from, to := &d.undo, &d.redo
	if redo {
		from, to = to, from
	}
	if len(*from) == 0 {
		return false
	}
	*to = append(*to, snapshot{slices.Clone(d.Text), slices.Clone(d.Marks)})
	s := (*from)[len(*from)-1]
	(*from)[len(*from)-1] = snapshot{}
	*from = (*from)[:len(*from)-1]
	d.Text, d.Marks = s.Text, s.Marks
	d.pending = nil
	d.trimHistory()
	d.invalidate()
	return true
}

func (d *Document) HTML() string {
	if !d.changed {
		return d.original
	}
	if !d.dirty {
		return d.html
	}
	if d.initial != nil && slices.Equal(d.Text, d.initial.Text) && slices.Equal(d.Marks, d.initial.Marks) {
		d.html = d.original
		d.dirty = false
		return d.html
	}
	var b strings.Builder
	list := uint8(0)
	for start := 0; start < len(d.Text); {
		end := start
		for end < len(d.Text) && d.Text[end] != '\n' {
			end++
		}
		f := d.Marks[start]
		if f.List != list {
			if list == 1 {
				b.WriteString("</ul>")
			} else if list == 2 {
				b.WriteString("</ol>")
			}
			if f.List == 1 {
				b.WriteString("<ul>")
			} else if f.List == 2 {
				b.WriteString("<ol>")
			}
			list = f.List
		}
		tag := "p"
		if list != 0 {
			tag = "li"
		} else if f.Heading > 0 {
			tag = fmt.Sprintf("h%d", min(f.Heading, 6))
		} else if f.Quote {
			tag = "blockquote"
		}
		b.WriteString("<" + tag + ">")
		if start == end {
			b.WriteString("<br>")
		}
		for i := start; i < end; {
			format := d.Marks[i]
			j := i + 1
			for j < end && d.Marks[j] == format {
				j++
			}
			if format.Link != "" {
				b.WriteString(`<a href="` + html.EscapeString(format.Link) + `">`)
			}
			for _, p := range tags {
				if format.Style&p.style != 0 {
					b.WriteString("<" + p.tag + ">")
				}
			}
			b.WriteString(html.EscapeString(string(d.Text[i:j])))
			for k := len(tags) - 1; k >= 0; k-- {
				p := tags[k]
				if format.Style&p.style != 0 {
					b.WriteString("</" + p.tag + ">")
				}
			}
			if format.Link != "" {
				b.WriteString("</a>")
			}
			i = j
		}
		b.WriteString("</" + tag + ">")
		start = end + 1
	}
	if list == 1 {
		b.WriteString("</ul>")
	} else if list == 2 {
		b.WriteString("</ol>")
	}
	d.html = b.String()
	d.dirty = false
	return d.html
}

var tags = []struct {
	style Style
	tag   string
}{{Bold, "strong"}, {Italic, "em"}, {Underline, "u"}, {Strike, "s"}, {Code, "code"}}

// Saved preserves editor content and formatting without rendering HTML on the
// UI thread. Undo history stays local; saved drafts carry only the current text.
type Saved struct {
	Original string
	Text     []rune
	Marks    []Format
	Changed  bool
}

func (d *Document) Save() Saved {
	s := Saved{Original: d.original, Changed: d.changed}
	if d.changed {
		s.Text = slices.Clone(d.Text)
		s.Marks = slices.Clone(d.Marks)
	}
	return s
}
func Restore(s Saved) *Document {
	d := Parse(s.Original)
	if s.Changed && len(s.Text) == len(s.Marks) {
		d.Text = slices.Clone(s.Text)
		d.Marks = slices.Clone(s.Marks)
		d.invalidate()
	}
	return d
}
