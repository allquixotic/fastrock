package platform

import (
	"encoding/json"
	"slices"
	"strings"
	"testing"
)

func TestTextSaveDialogOptions(t *testing.T) {
	name := "quoted\"\\\n世界.txt"
	quoted, _ := json.Marshal(name)
	script := textSaveScript(name)
	if !strings.Contains(script, "nameFieldStringValue = $("+string(quoted)+")") ||
		!strings.Contains(script, `allowedFileTypes = $(["txt","md","markdown"])`) ||
		!strings.Contains(script, "NSModalResponseOK") {
		t.Fatal("native save script lost quoting, default extension, or cancellation")
	}
	args := textSaveArgs(name)
	if !slices.Contains(args, "--filename="+name) || !slices.Contains(args, "--confirm-overwrite") ||
		!slices.Contains(args, "--file-filter=Text | *.txt") || !slices.Contains(args, "--file-filter=Markdown | *.md *.markdown") {
		t.Fatal("save dialog options lost their filename or filters")
	}
	if !strings.Contains(textSaveFilter, "Markdown\x00*.md;*.markdown\x00Text\x00*.txt\x00") || !strings.HasSuffix(textSaveFilter, "\x00\x00") {
		t.Fatal("Windows filter lacks paired/double-terminated entries")
	}
}
