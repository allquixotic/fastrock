package ui

import (
	"strings"
	"testing"
	"unicode/utf8"
)

func TestV26DiffMetadataCountsAndPaths(t *testing.T) {
	raw := `diff --git "a/old name.txt" "b/new name.txt"
similarity index 80%
rename from old name.txt
rename to new name.txt
--- "a/old name.txt"
+++ "b/new name.txt"
@@ -10,3 +20,3 @@ heading
 context
---literal content
+++literal content
 tail
\ No newline at end of file
diff --git a/deleted b/deleted
deleted file mode 100644
--- a/deleted
+++ /dev/null
@@ -1 +0,0 @@
-gone
diff --git a/picture.png b/picture.png
GIT binary patch
literal 32
ABCD
diff --git a/new b/new
new file mode 100644
--- /dev/null
+++ b/new
@@ -0,0 +1 @@
+created
`
	files, truncated := parseDiff(raw)
	if truncated || len(files) != 4 {
		t.Fatalf("files=%d truncated=%v", len(files), truncated)
	}
	f := files[0]
	if f.Path != "new name.txt" || f.OldPath != "old name.txt" || f.Status != "renamed" || f.Added != 1 || f.Removed != 1 {
		t.Fatalf("rename: %+v", f)
	}
	if len(f.Rows) != 6 || f.Rows[2].Kind != diffRemoved || f.Rows[2].Old != 11 || f.Rows[3].Kind != diffAdded || f.Rows[3].New != 21 || f.Rows[4].Old != 12 || f.Rows[4].New != 22 || f.Rows[5].Kind != diffNoNewline {
		t.Fatalf("hunk counters: %+v", f.Rows)
	}
	if raw[f.Rows[2].Start:f.Rows[2].End] != "--literal content" {
		t.Fatal("content confused with a file header")
	}
	if files[1].Status != "deleted" || files[1].Path != "" || files[1].Removed != 1 {
		t.Fatalf("delete: %+v", files[1])
	}
	if !files[2].Binary || len(files[2].Rows) != 1 || files[2].Rows[0].Kind != diffBinary {
		t.Fatalf("binary: %+v", files[2])
	}
	if files[3].Status != "added" || files[3].Added != 1 || files[3].Rows[1].New != 1 {
		t.Fatalf("add: %+v", files[3])
	}
	for _, f := range files {
		for _, l := range f.Rows {
			if l.Start < 0 || l.End > len(raw) || l.Start > l.End {
				t.Fatal("invalid source range")
			}
		}
	}
}
func TestV26PlainHunksAndQuotedPaths(t *testing.T) {
	for _, raw := range []string{"@@ -1 +1 @@\n-before\n+after\n", "--- a/one\told date\n+++ b/one\tnew date\n@@ -1 +1 @@\n-before\n+after\n--- a/two\n+++ b/two\n@@ -0,0 +1 @@\n+new\n"} {
		files, _ := parseDiff(raw)
		if len(files) == 0 || files[0].Added != 1 || files[0].Removed != 1 {
			t.Fatal(files)
		}
	}
	old, next := diffGitPaths(`"a/\346\227\245\346\234\254.txt" "b/\346\227\245\346\234\254.txt"`)
	if old != "日本.txt" || next != old {
		t.Fatalf("git octal paths: %q %q", old, next)
	}
	old, next = diffGitPaths("a/name b/with spaces b/name b/with spaces")
	if old != "name b/with spaces" || next != old {
		t.Fatalf("unquoted path spaces: %q %q", old, next)
	}
}
func TestV26IntralineChangesAndBudget(t *testing.T) {
	old, next := "hello 世界, old value + keep", "hello 地球, new value + keep"
	budget := 100000
	left, right := diffIntraline(old, next, &budget)
	if len(left) != 2 || len(right) != 2 || old[left[0].Start:left[0].End] != "世界" || next[right[0].Start:right[0].End] != "地球" || old[left[1].Start:left[1].End] != "old" || next[right[1].Start:right[1].End] != "new" {
		t.Fatalf("highlights: %+v %+v", left, right)
	}
	for _, pair := range []struct {
		text   string
		ranges []diffRange
	}{{old, left}, {next, right}} {
		for _, r := range pair.ranges {
			if !utf8.ValidString(pair.text[r.Start:r.End]) {
				t.Fatal("split Unicode highlight")
			}
		}
	}
	budget = 0
	if l, r := diffIntraline(old, next, &budget); len(l)+len(r) != 0 {
		t.Fatal("exceeded highlight budget")
	}
	budget = 100000
	if l, r := diffIntraline("unrelated words", "xyz", &budget); len(l)+len(r) != 0 {
		t.Fatal("highlighted unrelated lines")
	}
}
func TestV26LargeLineAndRowLimits(t *testing.T) {
	raw := "@@ -0,0 +1 @@\n+" + strings.Repeat("a\t🙂", 100000) + "\n"
	files, truncated := parseDiff(raw)
	if truncated {
		t.Fatal("single long line was truncated")
	}
	l := files[0].Rows[1]
	at := diffColumnByte(raw, &l, 450001)
	if at < 0 || at > l.End-l.Start || !utf8.ValidString(raw[l.Start:l.Start+at]) || diffByteColumn(raw, &l, at) > 450001 {
		t.Fatal("long-line index split Unicode or exceeded column")
	}
	huge := "@@ -0,0 +100005 @@\n" + strings.Repeat("+x\n", 100005)
	files, truncated = parseDiff(huge)
	if !truncated || len(files[0].Rows) != 100000 {
		t.Fatal("row bound missing")
	}
	bounded := boundedDiffText("first\ncut🙂line", 12)
	if bounded != "first\n" || !utf8.ValidString(bounded) {
		t.Fatalf("partial line retained: %q", bounded)
	}
}

func FuzzV26DiffRecords(f *testing.F) {
	f.Add("diff --git a/a b/a\n@@ -1 +1 @@\n-old 世界\n+new 地球\n")
	f.Add("--- a/a\n+++ b/a\n@@ -0,0 +1 @@\n+new\n")
	f.Add("@@ -999999999999999999 +0,0 @@\n-x\n")
	f.Fuzz(func(t *testing.T, source string) {
		if len(source) > 64<<10 {
			t.Skip()
		}
		files, _ := parseDiff(source)
		for _, file := range files {
			for _, line := range file.Rows {
				if line.Start < 0 || line.End < line.Start || line.End > len(source) {
					t.Fatal("invalid source range")
				}
				for _, r := range line.Emphasis {
					if r.Start < 0 || r.End < r.Start || r.End > line.End-line.Start {
						t.Fatal("invalid highlight range")
					}
				}
				for _, column := range []int{0, 1, line.Columns / 2, line.Columns, line.Columns + 1} {
					at := diffColumnByte(source, &line, column)
					if at < 0 || at > line.End-line.Start {
						t.Fatal("invalid column seek")
					}
				}
			}
		}
	})
}
