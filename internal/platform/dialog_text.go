package platform

import (
	"encoding/json"
	"fmt"
	"path/filepath"
	"strings"
)

const textSaveFilter = "Markdown\x00*.md;*.markdown\x00Text\x00*.txt\x00All files\x00*.*\x00\x00"

func textSaveExtension(name string) string {
	switch strings.ToLower(filepath.Ext(name)) {
	case ".txt":
		return "txt"
	case ".markdown":
		return "markdown"
	default:
		return "md"
	}
}

// JSON string literals preserve filenames as data even in an OSA program.
// AppKit supplies the native type filter and overwrite confirmation.
func textSaveScript(name string) string {
	quoted, _ := json.Marshal(name)
	types := []string{textSaveExtension(name)}
	for _, ext := range []string{"md", "txt", "markdown"} {
		if ext != types[0] {
			types = append(types, ext)
		}
	}
	allowed, _ := json.Marshal(types)
	return fmt.Sprintf(`ObjC.import('AppKit');
var app = $.NSApplication.sharedApplication;
var panel = $.NSSavePanel.savePanel;
panel.title = $('Save text');
panel.allowedFileTypes = $(%s);
panel.allowsOtherFileTypes = true;
panel.canCreateDirectories = true;
panel.nameFieldStringValue = $(%s);
app.activateIgnoringOtherApps(true);
panel.runModal == $.NSModalResponseOK ? ObjC.unwrap(panel.URL.path) : '';`, allowed, quoted)
}

func textSaveArgs(name string) []string {
	return []string{"--file-selection", "--save", "--confirm-overwrite", "--filename=" + name,
		"--file-filter=Markdown | *.md *.markdown", "--file-filter=Text | *.txt", "--file-filter=All files | *"}
}
