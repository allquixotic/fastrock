//go:build fltk_headless

package ui

import (
	"bytes"
	"testing"
	"unicode/utf8"
)

func FuzzCompleteUTF8Page(f *testing.F) {
	f.Add([]byte("a🌍"), true)
	f.Add([]byte{0xf0, 0x9f}, true)
	f.Fuzz(func(t *testing.T, data []byte, more bool) {
		if len(data) > 64<<10 {
			t.Skip()
		}
		out, err := completeUTF8Page(data, more)
		if err == nil && (!utf8.Valid(out) || !bytes.HasPrefix(data, out) || len(data)-len(out) > 3) {
			t.Fatal("invalid UTF-8 page boundary")
		}
	})
}

func FuzzMarkdownLayout(f *testing.F) {
	f.Add("**hello** [link](https://example.com)\n\n|a|b|\n|-|-|\n|1|2|")
	f.Fuzz(func(t *testing.T, source string) {
		if len(source) > 4096 {
			t.Skip()
		}
		layout := prepareTranscript(source, 400, 13)
		if !utf8.ValidString(layout.Plain) {
			t.Fatal("invalid rendered text")
		}
		for _, line := range layout.Lines {
			if line.Start < 0 || line.End < line.Start || line.End > len(layout.Plain) {
				t.Fatal("invalid selection bounds")
			}
		}
	})
}
