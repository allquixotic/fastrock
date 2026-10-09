package fltkdriver

import (
	"golang.org/x/mobile/event/key"
	"testing"
)

func TestNativeKeyTranslation(t *testing.T) {
	for _, tc := range []struct {
		native int
		want   key.Code
	}{
		{'a', key.CodeA}, {'Z', key.CodeZ}, {'0', key.Code0}, {'9', key.Code9},
		{0xff08, key.CodeDeleteBackspace}, {0xffff, key.CodeDeleteForward},
		{0xff51, key.CodeLeftArrow}, {0xff0d, key.CodeReturnEnter},
		{0xff8d, key.CodeKeypadEnter}, {0xffbe, key.CodeF1}, {0xffc9, key.CodeF12},
		{0x100000, key.CodeUnknown},
	} {
		if got := keyCode(tc.native); got != tc.want {
			t.Errorf("key %#x: got %v, want %v", tc.native, got, tc.want)
		}
	}
	if got := modifiers(0x10000 | 0x40000 | 0x80000 | 0x400000); got != key.ModShift|key.ModControl|key.ModAlt|key.ModMeta {
		t.Fatalf("modifier translation = %v", got)
	}
}
