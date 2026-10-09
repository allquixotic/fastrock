package codex

import (
	"bufio"
	"bytes"
	"errors"
	"testing"
)

func FuzzRPCFraming(f *testing.F) {
	f.Add([]byte("{\"id\":1,\"result\":{}}\n"))
	f.Add([]byte("{\"method\":\"a\",\"params\":\"\\\"\"}\n"))
	f.Fuzz(func(t *testing.T, data []byte) {
		if len(data) > 64<<10 {
			t.Skip()
		}
		r := bufio.NewReaderSize(bytes.NewReader(data), 64)
		frame, _, err := readFrameHeader(r, 4096)
		if len(frame) > 4096 {
			t.Fatal("frame exceeded admission limit")
		}
		if errors.Is(err, errFrameTooLarge) && len(frame) != 0 {
			t.Fatal("oversize frame retained")
		}
	})
}
