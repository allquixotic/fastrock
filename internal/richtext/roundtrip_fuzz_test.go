package richtext

import "testing"

func FuzzHTMLRoundTrip(f *testing.F) {
	f.Add("<p>Hello <b>world 🌍</b></p>")
	f.Add("<table><tr><td>&amp;</td></tr></table>")
	f.Fuzz(func(t *testing.T, source string) {
		if len(source) > 32<<10 {
			t.Skip()
		}
		d := Parse(source)
		if d.HTML() != source {
			t.Fatal("untouched HTML changed")
		}
		assertSpans(t, d)
		d.Sync(append(d.Text, 'x'))
		_ = Parse(d.HTML())
	})
}
