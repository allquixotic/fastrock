// Package richtext converts Rally HTML to an editable, styled Unicode document.
// It never executes HTML or fetches resources. Unedited HTML is returned verbatim.
package richtext

import (
	"crypto/sha256"
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

type Document struct {
	Unsupported         []string
	Revision            uint64
	Text                []rune
	spans, spareSpans   []Span
	original            string
	originalFingerprint [sha256.Size]byte
	changed             bool
	pending             *Format
	undo, redo          []*undoGroup
	history             *historyPool
	html                string
	dirty               bool
	lastEdit            time.Time
}

func Parse(source string) *Document {
	return ParseWithLinks(source, SafeLink)
}

// ParseWithLinks lets a read-only renderer retain local file citations while
// editable Rally documents continue to use the web-only SafeLink policy.
func ParseWithLinks(source string, link func(string) string) *Document {
	if link == nil {
		link = SafeLink
	}
	d := &Document{original: source}
	nodes, err := xhtml.ParseFragment(strings.NewReader(source), &xhtml.Node{Type: xhtml.ElementNode, Data: "div", DataAtom: atom.Div})
	if err != nil {
		d.Text = []rune(source)
		if len(d.Text) > 0 {
			d.spans = []Span{{End: len(d.Text)}}
		}
		return d
	}
	appendText := func(s string, f Format, pre bool) {
		for _, r := range s {
			if !pre && unicode.IsSpace(r) {
				if len(d.Text) == 0 || unicode.IsSpace(d.Text[len(d.Text)-1]) {
					continue
				}
				r = ' '
			}
			d.appendRune(r, f)
		}
	}
	newline := func(f Format) {
		for len(d.Text) > 0 && d.Text[len(d.Text)-1] == ' ' {
			d.truncate(len(d.Text) - 1)
		}
		if len(d.Text) > 0 && d.Text[len(d.Text)-1] != '\n' {
			d.appendRune('\n', f)
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
		unsupported := false
		switch tag {
		case "p", "div", "br", "span", "b", "strong", "em", "i", "u", "s", "del", "strike", "code", "pre", "blockquote", "ul", "ol", "li", "a", "h1", "h2", "h3", "h4", "h5", "h6":
		default:
			unsupported = true
		}
		for _, a := range n.Attr {
			if a.Key != "href" || tag != "a" {
				unsupported = true
			}
		}
		if unsupported && !slices.Contains(d.Unsupported, tag) {
			d.Unsupported = append(d.Unsupported, tag)
		}
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
					f.Link = link(a.Val)
				}
			}
		case "br":
			d.appendRune('\n', f)
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
		d.truncate(len(d.Text) - 1)
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

func (d *Document) invalidate() { d.changed = true; d.dirty = true; d.Revision++ }

// Sync reconciles a native editor insertion/deletion while retaining surrounding styles.
// Unchanged frames do no allocation.
func (d *Document) Sync(next []rune) bool {
	if slices.Equal(d.Text, next) {
		return false
	}
	prefix := 0
	for prefix < min(len(d.Text), len(next)) && d.Text[prefix] == next[prefix] {
		prefix++
	}
	suffix := 0
	for suffix < min(len(d.Text)-prefix, len(next)-prefix) && d.Text[len(d.Text)-1-suffix] == next[len(next)-1-suffix] {
		suffix++
	}
	return d.ApplyEdit(prefix, len(d.Text)-prefix-suffix, next[prefix:len(next)-suffix])
}

// ApplyEdit accepts a native editor's changed range without scanning unchanged
// text to rediscover its boundaries. Styles outside the edit retain identity.
func (d *Document) ApplyEdit(start, removed int, inserted []rune) bool {
	start = max(0, min(start, len(d.Text)))
	removed = max(0, min(removed, len(d.Text)-start))
	end := start + removed
	if slices.Equal(d.Text[start:end], inserted) {
		return false
	}
	d.captureOriginal()
	f := d.FormatAt(max(0, start-1))
	after := []Span(nil)
	if len(inserted) > 0 {
		after = []Span{{End: len(inserted), Format: f}}
	}
	e := editDelta{start: start, removed: removed, inserted: len(inserted), text: true,
		beforeText: slices.Clone(d.Text[start:end]), afterText: slices.Clone(inserted),
		beforeSpans: d.sliceSpans(start, end), afterSpans: after}
	d.record(e, true)
	d.applyDelta(e, false)
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
	format, _ := d.RunAt(i)
	return format
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
	remove := true
	for i := start; i < end; {
		f, next := d.RunAt(i)
		if f.Style&style == 0 {
			remove = false
			break
		}
		i = next
	}
	d.formatRange(start, end, func(f Format) Format {
		if remove {
			f.Style &^= style
		} else {
			f.Style |= style
		}
		return f
	})
}

func (d *Document) formatRange(start, end int, transform func(Format) Format) {
	if start == end {
		return
	}
	before := d.sliceSpans(start, end)
	var after []Span
	for _, s := range before {
		after = appendSpan(after, s.End, transform(s.Format))
	}
	if slices.Equal(before, after) {
		return
	}
	d.captureOriginal()
	e := editDelta{start: start, removed: end - start, inserted: end - start, beforeSpans: before, afterSpans: after}
	d.record(e, false)
	d.applyDelta(e, false)
	d.invalidate()
}

func (d *Document) Link(start, end int, link string) {
	start, end = d.Selection(start, end)
	link = SafeLink(link)
	d.formatRange(start, end, func(f Format) Format { f.Link = link; return f })
}

func (d *Document) Paragraph(start, end int, heading, list uint8, quote bool) {
	start, end = d.Selection(start, end)
	for start > 0 && d.Text[start-1] != '\n' {
		start--
	}
	for end < len(d.Text) && d.Text[end] != '\n' {
		end++
	}
	d.formatRange(start, end, func(f Format) Format { f.Heading, f.List, f.Quote = heading, list, quote; return f })
}

func (d *Document) HTML() string {
	if !d.changed {
		return d.original
	}
	if !d.dirty {
		return d.html
	}
	if d.fingerprint() == d.originalFingerprint {
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
		f, _ := d.RunAt(start)
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
			format, next := d.RunAt(i)
			j := min(end, next)
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
