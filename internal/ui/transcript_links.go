package ui

import (
	"html"
	"net/url"
	"path/filepath"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/yuin/goldmark/ast"
	"github.com/yuin/goldmark/renderer"
	gmtext "github.com/yuin/goldmark/text"
	"github.com/yuin/goldmark/util"
)

func externalTranscriptLink(target string) bool {
	u, err := url.Parse(target)
	if err != nil || strings.ContainsFunc(target, unicode.IsControl) {
		return false
	}
	switch strings.ToLower(u.Scheme) {
	case "http", "https":
		return u.Host != ""
	case "mailto":
		return u.Opaque != ""
	}
	return false
}

func transcriptSafeLink(target string) string {
	target = strings.TrimSpace(target)
	if externalTranscriptLink(target) {
		return target
	}
	if path, _, _ := fileLocation(target); path != "" {
		return target
	}
	return ""
}

// Keep the renderer's HTML/resource policy intact, changing only link targets.
// The editable rich-text parser continues to reject local links separately.
type transcriptLinkRenderer struct{}

func (transcriptLinkRenderer) RegisterFuncs(r renderer.NodeRendererFuncRegisterer) {
	r.Register(ast.KindLink, func(w util.BufWriter, _ []byte, n ast.Node, entering bool) (ast.WalkStatus, error) {
		if !entering {
			_, err := w.WriteString("</a>")
			return ast.WalkContinue, err
		}
		target := transcriptSafeLink(string(n.(*ast.Link).Destination))
		_, err := w.WriteString(`<a href="` + html.EscapeString(target) + `">`)
		return ast.WalkContinue, err
	})
}

func citationDestination(value string) string {
	if value == "" || len(value) > 300 || strings.ContainsFunc(value, unicode.IsSpace) || strings.ContainsAny(value, "()<>\"',;={} |*$`") || strings.Contains(value, "::") || strings.Contains(value, "://") {
		return ""
	}
	path, line, _ := fileLocation(value)
	if path == "" || strings.HasSuffix(path, "/") || strings.HasSuffix(path, "\\") {
		return ""
	}
	name := path[strings.LastIndexAny(path, "/\\")+1:]
	ext := filepath.Ext(name)
	if len(name) <= len(ext) || len(ext) < 2 || len(ext) > 9 || !strings.ContainsAny(path, "/\\") && line == 0 {
		return ""
	}
	hasLetter := false
	for _, r := range ext[1:] {
		if !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9') {
			return ""
		}
		hasLetter = hasLetter || r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z'
	}
	if !hasLetter {
		return ""
	}
	return value
}

func transcriptAutolinks(text string) []transcriptSourceRange {
	var ranges []transcriptSourceRange
	for i := 0; i < len(text); {
		start := i
		for i < len(text) {
			r, size := utf8.DecodeRuneInString(text[i:])
			if unicode.IsSpace(r) {
				break
			}
			i += size
		}
		end := i
		if start == end {
			_, size := utf8.DecodeRuneInString(text[i:])
			i += size
			continue
		}
		for start < end && strings.ContainsRune("([\"'", rune(text[start])) {
			start++
		}
		for end > start {
			last := text[end-1]
			if strings.ContainsRune(".,;:!?\"']}", rune(last)) || last == ')' && strings.Count(text[start:end], "(") < strings.Count(text[start:end], ")") {
				end--
			} else {
				break
			}
		}
		value := text[start:end]
		if citationDestination(value) != "" || strings.HasPrefix(strings.ToLower(value), "mailto:") && externalTranscriptLink(value) {
			ranges = append(ranges, transcriptSourceRange{start, end})
		}
	}
	return ranges
}

func addTranscriptAutolinks(root ast.Node, source []byte) {
	var nodes []ast.Node
	_ = ast.Walk(root, func(n ast.Node, entering bool) (ast.WalkStatus, error) {
		if !entering {
			return ast.WalkContinue, nil
		}
		switch n.Kind() {
		case ast.KindCodeBlock, ast.KindFencedCodeBlock, ast.KindLink, ast.KindAutoLink, ast.KindRawHTML, ast.KindHTMLBlock, ast.KindImage:
			return ast.WalkSkipChildren, nil
		case ast.KindCodeSpan:
			nodes = append(nodes, n)
			return ast.WalkSkipChildren, nil
		case ast.KindText:
			nodes = append(nodes, n)
		}
		return ast.WalkContinue, nil
	})
	for _, n := range nodes {
		parent := n.Parent()
		if n.Kind() == ast.KindCodeSpan {
			var b strings.Builder
			for c := n.FirstChild(); c != nil; c = c.NextSibling() {
				if text, ok := c.(*ast.Text); ok {
					b.Write(text.Value(source))
				}
			}
			if destination := citationDestination(b.String()); destination != "" {
				link := ast.NewLink()
				link.Destination = []byte(destination)
				parent.InsertBefore(parent, n, link)
				parent.RemoveChild(parent, n)
				link.AppendChild(link, n)
			}
			continue
		}
		text := n.(*ast.Text)
		ranges := transcriptAutolinks(string(text.Value(source)))
		if len(ranges) == 0 || text.Segment.Padding != 0 {
			continue
		}
		start := 0
		var last *ast.Text
		appendText := func(from, to int, container ast.Node) {
			last = ast.NewTextSegment(gmtext.NewSegment(text.Segment.Start+from, text.Segment.Start+to))
			last.SetRaw(text.IsRaw())
			if container == parent {
				parent.InsertBefore(parent, n, last)
			} else {
				container.AppendChild(container, last)
			}
		}
		for _, r := range ranges {
			if r.start > start {
				appendText(start, r.start, parent)
			}
			link := ast.NewLink()
			link.Destination = append([]byte(nil), source[text.Segment.Start+r.start:text.Segment.Start+r.end]...)
			parent.InsertBefore(parent, n, link)
			appendText(r.start, r.end, link)
			start = r.end
		}
		if start < text.Segment.Len() {
			appendText(start, text.Segment.Len(), parent)
		}
		last.SetSoftLineBreak(text.SoftLineBreak())
		last.SetHardLineBreak(text.HardLineBreak())
		parent.RemoveChild(parent, n)
	}
}
