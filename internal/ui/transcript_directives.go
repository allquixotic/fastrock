package ui

import (
	"fmt"
	"net/url"
	"slices"
	"strconv"
	"strings"

	"github.com/yuin/goldmark"
	"github.com/yuin/goldmark/ast"
	gmtext "github.com/yuin/goldmark/text"
)

type transcriptDirective struct {
	name, label string
	attributes  map[string]string
	length      int
}

func directiveName(c byte) bool {
	return c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '-' || c == '_'
}

func parseTranscriptDirective(source string) (d transcriptDirective, valid bool) {
	d.attributes = map[string]string{}
	i := 0
	defer func() {
		if !valid {
			d.length = i
		}
	}()
	for i < len(source) && source[i] == ':' {
		i++
	}
	if i < 1 || i > 3 || i == len(source) || !(source[i] >= 'a' && source[i] <= 'z' || source[i] >= 'A' && source[i] <= 'Z') {
		return d, false
	}
	start := i
	for i < len(source) && directiveName(source[i]) {
		i++
	}
	d.name = source[start:i]
	if i < len(source) && source[i] == '[' {
		start, i = i+1, i+1
		depth, escaped := 1, false
		for i < len(source) && depth > 0 {
			c := source[i]
			if c == '\n' || c == '\r' {
				return d, false
			}
			if escaped {
				escaped = false
			} else if c == '\\' {
				escaped = true
			} else if c == '[' {
				depth++
			} else if c == ']' {
				depth--
			}
			i++
		}
		if depth != 0 {
			return d, false
		}
		d.label = source[start : i-1]
	}
	if i >= len(source) || source[i] != '{' {
		return d, false
	}
	i++
	for i < len(source) {
		for i < len(source) && (source[i] == ' ' || source[i] == '\t') {
			i++
		}
		if i < len(source) && source[i] == '}' {
			d.length = i + 1
			return d, true
		}
		start = i
		for i < len(source) && directiveName(source[i]) {
			i++
		}
		if start == i {
			return d, false
		}
		key := source[start:i]
		if _, duplicate := d.attributes[key]; duplicate {
			return d, false
		}
		for i < len(source) && (source[i] == ' ' || source[i] == '\t') {
			i++
		}
		if i == len(source) || source[i] != '=' {
			return d, false
		}
		i++
		for i < len(source) && (source[i] == ' ' || source[i] == '\t') {
			i++
		}
		if i == len(source) {
			return d, false
		}
		if source[i] == '"' || source[i] == '\'' {
			quote := source[i]
			i++
			var b strings.Builder
			for i < len(source) && source[i] != quote {
				if source[i] == '\r' || source[i] == '\n' {
					return d, false
				}
				if source[i] == '\\' && i+1 < len(source) && (source[i+1] == quote || source[i+1] == '\\') {
					i++
				}
				b.WriteByte(source[i])
				i++
			}
			if i == len(source) {
				return d, false
			}
			d.attributes[key] = b.String()
			i++
		} else {
			start = i
			for i < len(source) && !strings.ContainsRune(" \t\r\n}", rune(source[i])) {
				i++
			}
			if start == i {
				return d, false
			}
			d.attributes[key] = source[start:i]
		}
	}
	return d, false
}

type transcriptSourceRange struct{ start, end int }

// Let the Markdown parser identify literal regions, including indented and
// nested fences. A string scan alone mistakes directives in examples for UI.
func transcriptLiteralRanges(source string) []transcriptSourceRange {
	data := []byte(source)
	root := goldmark.New().Parser().Parse(gmtext.NewReader(data))
	var ranges []transcriptSourceRange
	_ = ast.Walk(root, func(n ast.Node, entering bool) (ast.WalkStatus, error) {
		if !entering {
			return ast.WalkContinue, nil
		}
		switch n.Kind() {
		case ast.KindCodeBlock, ast.KindFencedCodeBlock:
			for i := range n.Lines().Len() {
				s := n.Lines().At(i)
				ranges = append(ranges, transcriptSourceRange{s.Start, s.Stop})
			}
			return ast.WalkSkipChildren, nil
		case ast.KindCodeSpan:
			for child := n.FirstChild(); child != nil; child = child.NextSibling() {
				if t, ok := child.(*ast.Text); ok {
					ranges = append(ranges, transcriptSourceRange{t.Segment.Start, t.Segment.Stop})
				}
			}
			return ast.WalkSkipChildren, nil
		}
		return ast.WalkContinue, nil
	})
	slices.SortFunc(ranges, func(a, b transcriptSourceRange) int { return a.start - b.start })
	return ranges
}

func transcriptMarkdownText(s string) string {
	return strings.NewReplacer("\\", "\\\\", "`", "\\`", "*", "\\*", "_", "\\_", "[", "\\[", "]", "\\]", "<", "&lt;", ">", "&gt;", "#", "\\#", "!", "\\!").Replace(s)
}

func transcriptCitation(path, label string) string {
	if label == "" {
		label = path
	}
	// Encode Markdown delimiters/backslashes without changing the stored path.
	return "[" + transcriptMarkdownText(label) + "](<" + url.PathEscape(path) + ">)"
}

func renderTranscriptDirective(d transcriptDirective, standalone bool) (string, bool) {
	switch d.name {
	case "codex-file-citation":
		if path := d.attributes["path"]; path != "" {
			return transcriptCitation(path, path), true
		}
	case "codex-followup":
		if d.label != "" {
			return transcriptMarkdownText(d.label), true
		}
	case "git-stage", "git-commit", "git-create-branch", "git-push", "git-create-pr", "codex-inline-vis":
		return "", true
	case "code-comment":
		title, body, file := strings.TrimSpace(d.attributes["title"]), strings.TrimSpace(d.attributes["body"]), strings.TrimSpace(d.attributes["file"])
		if !standalone || title == "" || body == "" || file == "" {
			return "", false
		}
		number := func(key string, fallback int) int {
			n, err := strconv.Atoi(strings.TrimLeft(d.attributes[key], "Pp"))
			if err != nil || n < 1 {
				return fallback
			}
			return n
		}
		start := number("start", 1)
		end := max(start, number("end", start))
		priority, err := strconv.Atoi(strings.TrimLeft(d.attributes["priority"], "Pp"))
		if err == nil && priority >= 0 && priority <= 3 && !(len(title) >= 4 && (strings.HasPrefix(title, "[P") || strings.HasPrefix(title, "[p")) && title[3] == ']') {
			title = fmt.Sprintf("[P%d] %s", priority, title)
		}
		location := fmt.Sprintf("%s:%d", file, start)
		if end > start {
			location += fmt.Sprintf("-%d", end)
		}
		return "- **" + transcriptMarkdownText(title) + "** — " + transcriptCitation(location, location) + "\n  " + body, true
	}
	return "", false
}

func visibleTranscriptMarkdown(source string) string {
	if !strings.Contains(source, ":") {
		return source
	}
	ranges := transcriptLiteralRanges(source)
	var out strings.Builder
	out.Grow(len(source))
	rangeIndex := 0
	lineStart := 0
	for i := 0; i < len(source); {
		for rangeIndex < len(ranges) && ranges[rangeIndex].end <= i {
			rangeIndex++
		}
		if source[i] == ':' && (i == 0 || source[i-1] != ':' && source[i-1] != '\\') && (rangeIndex == len(ranges) || i < ranges[rangeIndex].start) {
			if d, ok := parseTranscriptDirective(source[i:]); ok {
				if rendered, handled := renderTranscriptDirective(d, strings.TrimSpace(source[lineStart:i]) == ""); handled {
					out.WriteString(rendered)
					i += d.length
					continue
				}
			} else if d.length > 1 {
				// A malformed candidate stays literal. Consume its scanned prefix
				// once so nested, unterminated attributes cannot cause rescans.
				out.WriteString(source[i : i+d.length])
				i += d.length
				continue
			}
		}
		out.WriteByte(source[i])
		if source[i] == '\n' {
			lineStart = i + 1
		}
		i++
	}
	return out.String()
}
