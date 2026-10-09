package ui

import (
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"
)

type diffKind uint8

const (
	diffMeta diffKind = iota
	diffHunk
	diffContext
	diffAdded
	diffRemoved
	diffNoNewline
	diffBinary
)

type diffRange struct{ Start, End int }
type diffColumn struct{ Byte, Column int }
type diffLine struct {
	Kind       diffKind
	Start, End int // byte offsets into the one immutable source
	Old, New   int
	Emphasis   []diffRange // byte offsets relative to displayed content
	Columns    int
	Index      []diffColumn // sparse seek points for long lines
}
type diffFile struct {
	Path, OldPath, Status string
	Text                  string
	Start                 int
	Rows                  []diffLine
	Added, Removed        int
	Binary, Collapsed     bool
	HunkSeen              bool
}
type diffSelection struct {
	Anchor, End     int
	Dragging, Focus bool
}

type diffParser struct {
	source                      string
	files                       []diffFile
	file                        int
	old, next, oldLeft, newLeft int
	inHunk, binary              bool
	rowCount                    int
	truncated                   bool
}

// Git's metadata is interpreted only outside a counted hunk. A source line
// starting with --- or +++ inside a hunk is still a removal or addition.
func parseDiff(source string) ([]diffFile, bool) {
	p := diffParser{source: source, file: -1}
	for start := 0; start < len(source); {
		end := start
		if i := strings.IndexByte(source[start:], '\n'); i >= 0 {
			end += i
		} else {
			end = len(source)
		}
		if p.rowCount >= 100000 {
			p.truncated = true
			break
		}
		p.line(start, end)
		start = end + 1
	}
	if p.file >= 0 {
		p.files[p.file].Text = source[p.files[p.file].Start:]
	}
	budget := 8_000_000
	for i := range p.files {
		f := &p.files[i]
		if f.Status == "modified" && f.Path != "" && f.OldPath != "" && f.Path != f.OldPath {
			f.Status = "renamed"
		}
		if f.Status == "deleted" {
			f.Path = ""
		}
		for j := range f.Rows {
			prepareDiffColumns(source, &f.Rows[j])
		}
		emphasizeDiff(source, f.Rows, &budget)
	}
	return p.files, p.truncated
}
func (p *diffParser) startFile(start int) {
	if p.file >= 0 {
		f := &p.files[p.file]
		f.Text = p.source[f.Start:start]
	}
	p.files = append(p.files, diffFile{Start: start, Status: "modified"})
	p.file = len(p.files) - 1
	p.inHunk, p.binary = false, false
}
func (p *diffParser) add(kind diffKind, start, end, old, next int) {
	if p.file < 0 {
		p.startFile(start)
	}
	p.files[p.file].Rows = append(p.files[p.file].Rows, diffLine{Kind: kind, Start: start, End: end, Old: old, New: next})
	p.rowCount++
}
func (p *diffParser) line(start, end int) {
	line := strings.TrimSuffix(p.source[start:end], "\r")
	end = start + len(line)
	if p.inHunk && (p.oldLeft > 0 || p.newLeft > 0) {
		switch {
		case (line == "" || line[0] == ' ') && p.oldLeft > 0 && p.newLeft > 0:
			if line != "" {
				start++
			}
			p.add(diffContext, start, end, p.old, p.next)
			p.old++
			p.next++
			p.oldLeft--
			p.newLeft--
			return
		case len(line) > 0 && line[0] == '-' && p.oldLeft > 0:
			p.add(diffRemoved, start+1, end, p.old, 0)
			p.files[p.file].Removed++
			p.old++
			p.oldLeft--
			return
		case len(line) > 0 && line[0] == '+' && p.newLeft > 0:
			p.add(diffAdded, start+1, end, 0, p.next)
			p.files[p.file].Added++
			p.next++
			p.newLeft--
			return
		}
	}
	if strings.HasPrefix(line, `\ `) {
		p.add(diffNoNewline, start, end, 0, 0)
		return
	}
	p.inHunk = false
	if rest, ok := strings.CutPrefix(line, "diff --git "); ok {
		p.startFile(start)
		p.files[p.file].OldPath, p.files[p.file].Path = diffGitPaths(rest)
		return
	}
	if p.binary {
		return
	}
	if rest, ok := strings.CutPrefix(line, "--- "); ok {
		if p.file < 0 || p.files[p.file].HunkSeen {
			p.startFile(start)
		}
		p.files[p.file].OldPath = diffHeaderPath(rest)
		if p.files[p.file].OldPath == "" {
			p.files[p.file].Status = "added"
		}
		return
	}
	if rest, ok := strings.CutPrefix(line, "+++ "); ok && p.file >= 0 && !p.files[p.file].HunkSeen {
		p.files[p.file].Path = diffHeaderPath(rest)
		if p.files[p.file].Path == "" {
			p.files[p.file].Status = "deleted"
		}
		return
	}
	if old, oldCount, next, newCount, ok := diffHunkHeader(line); ok {
		p.add(diffHunk, start, end, 0, 0)
		p.files[p.file].HunkSeen = true
		p.old, p.oldLeft, p.next, p.newLeft = old, oldCount, next, newCount
		p.inHunk = true
		return
	}
	if p.file >= 0 && !p.files[p.file].HunkSeen {
		f := &p.files[p.file]
		switch {
		case strings.HasPrefix(line, "new file mode "):
			f.Status = "added"
			return
		case strings.HasPrefix(line, "deleted file mode "):
			f.Status = "deleted"
			return
		case strings.HasPrefix(line, "rename from "):
			f.OldPath = diffUnquote(strings.TrimPrefix(line, "rename from "))
			f.Status = "renamed"
			return
		case strings.HasPrefix(line, "rename to "):
			f.Path = diffUnquote(strings.TrimPrefix(line, "rename to "))
			f.Status = "renamed"
			return
		case strings.HasPrefix(line, "copy from "):
			f.OldPath = diffUnquote(strings.TrimPrefix(line, "copy from "))
			f.Status = "copied"
			return
		case strings.HasPrefix(line, "copy to "):
			f.Path = diffUnquote(strings.TrimPrefix(line, "copy to "))
			f.Status = "copied"
			return
		case strings.HasPrefix(line, "index "), strings.HasPrefix(line, "old mode "), strings.HasPrefix(line, "new mode "), strings.HasPrefix(line, "similarity index "), strings.HasPrefix(line, "dissimilarity index "):
			return
		}
	}
	if line == "GIT binary patch" || strings.HasPrefix(line, "Binary files ") && strings.HasSuffix(line, " differ") {
		p.add(diffBinary, start, end, 0, 0)
		p.files[p.file].Binary = true
		p.binary = line == "GIT binary patch"
		return
	}
	if line != "" {
		p.add(diffMeta, start, end, 0, 0)
	}
}
func diffHunkHeader(line string) (old, oldCount, next, newCount int, ok bool) {
	rest, ok := strings.CutPrefix(line, "@@ ")
	if !ok {
		return
	}
	header, _, ok := strings.Cut(rest, " @@")
	if !ok {
		return
	}
	fields := strings.Fields(header)
	if len(fields) != 2 || !strings.HasPrefix(fields[0], "-") || !strings.HasPrefix(fields[1], "+") {
		ok = false
		return
	}
	parse := func(s string) (int, int, bool) {
		a, b, found := strings.Cut(s, ",")
		n, e := strconv.ParseUint(a, 10, 31)
		m := uint64(1)
		var e2 error
		if found {
			m, e2 = strconv.ParseUint(b, 10, 31)
		}
		return int(n), int(m), e == nil && e2 == nil && n+m <= 1<<31-1
	}
	var ok1, ok2 bool
	old, oldCount, ok1 = parse(fields[0][1:])
	next, newCount, ok2 = parse(fields[1][1:])
	ok = ok1 && ok2
	return
}
func diffUnquote(path string) string {
	if s, e := strconv.Unquote(path); e == nil {
		return s
	}
	return path
}
func diffHeaderPath(path string) string {
	path, _, _ = strings.Cut(path, "\t")
	path = diffUnquote(strings.TrimSpace(path))
	if path == "/dev/null" {
		return ""
	}
	return stripDiffPrefix(path)
}
func stripDiffPrefix(path string) string {
	if strings.HasPrefix(path, "a/") || strings.HasPrefix(path, "b/") {
		return path[2:]
	}
	return path
}
func diffGitPaths(rest string) (string, string) {
	if strings.HasPrefix(rest, `"`) {
		escaped := false
		for i := 1; i < len(rest); i++ {
			if rest[i] == '"' && !escaped {
				return stripDiffPrefix(diffUnquote(rest[:i+1])), stripDiffPrefix(diffUnquote(strings.TrimSpace(rest[i+1:])))
			}
			if rest[i] == '\\' && !escaped {
				escaped = true
			} else {
				escaped = false
			}
		}
	}
	split := -1
	for from := 0; from < len(rest); {
		j := strings.Index(rest[from:], " b/")
		if j < 0 {
			break
		}
		j += from
		split = j
		if stripDiffPrefix(rest[:j]) == rest[j+3:] {
			break
		}
		from = j + 3
	}
	if split < 0 {
		return stripDiffPrefix(rest), ""
	}
	return stripDiffPrefix(rest[:split]), rest[split+3:]
}
func (f *diffFile) displayPath() string {
	if f.Path != "" && f.OldPath != "" && f.Path != f.OldPath {
		return f.OldPath + " → " + f.Path
	}
	if f.Path != "" {
		return f.Path
	}
	if f.OldPath != "" {
		return f.OldPath
	}
	return "(unnamed file)"
}

func prepareDiffColumns(source string, l *diffLine) {
	column := 0
	last := 0
	for i, r := range source[l.Start:l.End] {
		if column-last >= 128 {
			l.Index = append(l.Index, diffColumn{i, column})
			last = column
		}
		if r == '\t' {
			column += 4
		} else {
			column++
		}
	}
	l.Columns = column
}
func diffColumnByte(source string, l *diffLine, column int) int {
	start, col := 0, 0
	// Sparse indexes keep a huge single line from being rescanned while scrolling.
	lo, hi := 0, len(l.Index)
	for lo < hi {
		m := (lo + hi) / 2
		if l.Index[m].Column <= column {
			lo = m + 1
		} else {
			hi = m
		}
	}
	if lo > 0 {
		start, col = l.Index[lo-1].Byte, l.Index[lo-1].Column
	}
	for start < l.End-l.Start && col < column {
		r, n := utf8.DecodeRuneInString(source[l.Start+start : l.End])
		width := 1
		if r == '\t' {
			width = 4
		}
		if col+width > column {
			break
		}
		start += n
		col += width
	}
	return start
}
func diffTokens(s string) []diffRange {
	var out []diffRange
	start, previous := 0, -1
	for i, r := range s {
		class := 2
		if unicode.IsLetter(r) || unicode.IsNumber(r) || r == '_' {
			class = 0
		} else if unicode.IsSpace(r) {
			class = 1
		}
		if previous >= 0 && (previous == 2 || class != previous) {
			out = append(out, diffRange{start, i})
			start = i
		}
		previous = class
	}
	if len(s) > 0 {
		out = append(out, diffRange{start, len(s)})
	}
	return out
}
func appendDiffRange(ranges []diffRange, r diffRange) []diffRange {
	if len(ranges) > 0 && ranges[len(ranges)-1].End == r.Start {
		ranges[len(ranges)-1].End = r.End
		return ranges
	}
	return append(ranges, r)
}
func diffIntraline(old, next string, budget *int) ([]diffRange, []diffRange) {
	if old == next || len(old) > 2000 || len(next) > 2000 {
		return nil, nil
	}
	a, b := diffTokens(old), diffTokens(next)
	cost := (len(a) + 1) * (len(b) + 1)
	if len(a) > 256 || len(b) > 256 || cost > *budget {
		return nil, nil
	}
	*budget -= cost
	table := make([]uint16, cost)
	width := len(b) + 1
	for i := len(a) - 1; i >= 0; i-- {
		for j := len(b) - 1; j >= 0; j-- {
			if old[a[i].Start:a[i].End] == next[b[j].Start:b[j].End] {
				table[i*width+j] = 1 + table[(i+1)*width+j+1]
			} else {
				table[i*width+j] = max(table[(i+1)*width+j], table[i*width+j+1])
			}
		}
	}
	var left, right []diffRange
	i, j := 0, 0
	for i < len(a) || j < len(b) {
		if i < len(a) && j < len(b) && old[a[i].Start:a[i].End] == next[b[j].Start:b[j].End] {
			i++
			j++
		} else if j == len(b) || i < len(a) && table[(i+1)*width+j] >= table[i*width+j+1] {
			left = appendDiffRange(left, a[i])
			i++
		} else {
			right = appendDiffRange(right, b[j])
			j++
		}
	}
	changed := func(ranges []diffRange) int {
		n := 0
		for _, r := range ranges {
			n += r.End - r.Start
		}
		return n
	}
	if changed(left)*10 > len(old)*6 && changed(right)*10 > len(next)*6 {
		return nil, nil
	}
	return left, right
}
func emphasizeDiff(source string, rows []diffLine, budget *int) {
	for i := 0; i < len(rows); {
		if rows[i].Kind != diffRemoved {
			i++
			continue
		}
		start := i
		for i < len(rows) && rows[i].Kind == diffRemoved {
			i++
		}
		added := i
		for i < len(rows) && rows[i].Kind == diffAdded {
			i++
		}
		if added-start+i-added > 200 {
			continue
		}
		for j := 0; j < min(added-start, i-added); j++ {
			a, b := &rows[start+j], &rows[added+j]
			a.Emphasis, b.Emphasis = diffIntraline(source[a.Start:a.End], source[b.Start:b.End], budget)
		}
	}
}
