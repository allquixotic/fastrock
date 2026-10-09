package ui

import (
	"image"
	"image/png"
	"os"
	"path/filepath"
	"testing"
)

func TestAttachmentsDistinguishImagesAndFileMentions(t *testing.T) {
	dir := t.TempDir()
	imagePath := filepath.Join(dir, "image with spaces.png")
	f, err := os.Create(imagePath)
	if err != nil {
		t.Fatal(err)
	}
	if err = png.Encode(f, image.NewRGBA(image.Rect(0, 0, 2, 2))); err != nil {
		t.Fatal(err)
	}
	f.Close()
	textPath := filepath.Join(dir, "notes.txt")
	if err = os.WriteFile(textPath, []byte("notes"), 0600); err != nil {
		t.Fatal(err)
	}
	inputs, err := attachmentInputs([]string{imagePath, textPath})
	if err != nil {
		t.Fatal(err)
	}
	if len(inputs) != 2 || inputs[0]["type"] != "localImage" || inputs[0]["path"] != imagePath || inputs[1]["type"] != "text" {
		t.Fatalf("invalid attachment types: %#v", inputs)
	}
}

func TestInvalidAttachmentsAreNotSubmittedAsImages(t *testing.T) {
	dir := t.TempDir()
	bad := filepath.Join(dir, "not-an-image.png")
	if err := os.WriteFile(bad, []byte("plain text"), 0600); err != nil {
		t.Fatal(err)
	}
	large := filepath.Join(dir, "large.txt")
	f, err := os.Create(large)
	if err != nil {
		t.Fatal(err)
	}
	err = f.Truncate(maxAttachmentBytes + 1)
	f.Close()
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{dir, bad, large, filepath.Join(dir, "missing")} {
		if _, err := attachmentInputs([]string{path}); err == nil {
			t.Fatalf("accepted invalid attachment %s", path)
		}
	}
}
