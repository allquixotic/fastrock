package rally

import (
	"strings"
	"testing"
)

func FuzzQueryComposition(f *testing.F) {
	f.Add(`name" OR (Blocked = true)`, `a\b`)
	f.Fuzz(func(t *testing.T, a, b string) {
		if len(a)+len(b) > 64<<10 {
			t.Skip()
		}
		quoted := Quote(a)
		escaped := false
		for _, ch := range quoted[1 : len(quoted)-1] {
			if escaped {
				escaped = false
				continue
			}
			if ch == '"' || ch == '\n' || ch == '\r' {
				t.Fatal("unescaped query delimiter")
			}
			escaped = ch == '\\'
		}
		if escaped {
			t.Fatal("unterminated query escape")
		}
		combined := And(a, b)
		if !strings.Contains(combined, a) || !strings.Contains(combined, b) {
			t.Fatal("lost query operand")
		}
	})
}
