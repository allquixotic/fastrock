package codex

import (
	"bufio"
	"errors"
	"strings"
	"testing"
)

func TestOversizeFrameDrainsWithoutLosingFollowingReply(t *testing.T) {
	r := bufio.NewReaderSize(strings.NewReader(strings.Repeat("x", 4096)+"\n{\"id\":1,\"result\":true}\n"), 64)
	if _, err := readFrame(r, 256); !errors.Is(err, errFrameTooLarge) {
		t.Fatalf("oversize frame: %v", err)
	}
	data, err := readFrame(r, 256)
	if err != nil || string(data) != "{\"id\":1,\"result\":true}\n" {
		t.Fatalf("following reply lost: %q %v", data, err)
	}
}
func TestDiagnosticTailIsBounded(t *testing.T) {
	var b tailBuffer
	b.Write([]byte(strings.Repeat("a", 20000)))
	b.Write([]byte("ending"))
	if len(b.data) != 8192 || !strings.HasSuffix(b.message(), "ending") {
		t.Fatal("tail unbounded or incorrect")
	}
}

func TestOversizedRequestEnvelopeAfterParams(t *testing.T) {
	for _, raw := range []string{
		`{"params":{"text":"` + strings.Repeat("x", 8192) + `"},"id":42,"method":"approval"}`,
		`{"id":"quoted\\\"id","method":"approval","params":[{"nested":{"id":"wrong","method":"wrong"},"text":"` + strings.Repeat("x", 8192) + `"}]}`,
	} {
		reader := bufio.NewReaderSize(strings.NewReader(raw+"\n"), 64)
		_, header, err := readFrameHeader(reader, 256)
		if !errors.Is(err, errFrameTooLarge) || header.Method != "approval" || len(header.ID) == 0 {
			t.Fatalf("lost envelope: %#v %v", header, err)
		}
	}
}
