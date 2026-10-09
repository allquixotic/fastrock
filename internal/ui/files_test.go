package ui

import "testing"

func TestFindTextUnicodeAndWrap(t *testing.T) {
	for _, tc := range []struct {
		s, q  string
		start int
		back  bool
		a, b  int
	}{
		{"a 🚀 hello HELLO", "hello", 0, false, 4, 9},
		{"a 🚀 hello HELLO", "hello", 9, false, 10, 15},
		{"a 🚀 hello HELLO", "hello", 15, false, 4, 9},
		{"a 🚀 hello HELLO", "hello", 10, true, 4, 9},
		{"a 🚀 hello HELLO", "hello", 0, true, 10, 15},
		{"abc", "missing", 0, false, -1, -1},
	} {
		a, b := findText(tc.s, tc.q, tc.start, tc.back)
		if a != tc.a || b != tc.b {
			t.Fatalf("%+v: %d,%d", tc, a, b)
		}
	}
}

func TestUTF8PageBoundary(t *testing.T) {
	data := []byte("hello 🚀")
	for n := len(data) - 3; n < len(data); n++ {
		page, err := completeUTF8Page(data[:n], true)
		if err != nil || string(page) != "hello " {
			t.Fatalf("boundary %d: %q %v", n, page, err)
		}
	}
	if _, err := completeUTF8Page([]byte{'a', 0xff, 'b'}, true); err == nil {
		t.Fatal("invalid text accepted")
	}
}

func TestFileLinksAndLinePositions(t *testing.T) {
	for _, tc := range []struct {
		link, path   string
		line, column int
	}{
		{"src/main.go:42:3", "src/main.go", 42, 3},
		{"file:///C:/work/a%20b.go#L12C4", "C:/work/a b.go", 12, 4},
		{"/tmp/a.go#L5-L9", "/tmp/a.go", 5, 1},
		{`C:\work\a.go:8`, `C:\work\a.go`, 8, 1},
	} {
		path, line, column := fileLocation(tc.link)
		if path != tc.path || line != tc.line || column != tc.column {
			t.Fatalf("%s => %s:%d:%d", tc.link, path, line, column)
		}
	}
	buf := []rune("first\n🚀 hello\nlast")
	if pos, found := fileLinePosition(buf, 2, 3); !found || pos != 8 {
		t.Fatalf("%d %v", pos, found)
	}
	if pos, found := fileLinePosition(buf, 90, 1); found || pos != len(buf) {
		t.Fatalf("%d %v", pos, found)
	}
}
