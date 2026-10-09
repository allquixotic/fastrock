package update

import (
	"archive/zip"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func FuzzUpdateArchivePaths(f *testing.F) {
	f.Add("Fastrock.app/Contents/MacOS/fastrock", []byte("payload"))
	f.Add("../escape", []byte("payload"))
	f.Fuzz(func(t *testing.T, name string, data []byte) {
		if len(name) > 512 || len(data) > 4096 {
			t.Skip()
		}
		root := t.TempDir()
		archive := filepath.Join(root, "update.zip")
		file, err := os.Create(archive)
		if err != nil {
			t.Fatal(err)
		}
		writer := zip.NewWriter(file)
		entry, err := writer.Create(name)
		if err == nil {
			_, err = entry.Write(data)
		}
		closeErr := writer.Close()
		file.Close()
		if err != nil || closeErr != nil {
			return
		}
		dest := filepath.Join(root, "payload")
		err = extract(archive, dest, "darwin")
		unsafe := strings.ContainsAny(name, "\\:") || strings.HasPrefix(name, "/") || filepath.ToSlash(filepath.Clean(name)) != strings.TrimSuffix(name, "/") || name == "." || strings.HasPrefix(name, "../") || name != "Fastrock.app/" && !strings.HasPrefix(name, "Fastrock.app/")
		if unsafe && err == nil {
			t.Fatal("accepted unsafe archive path")
		}
		entries, e := os.ReadDir(root)
		if e != nil {
			t.Fatal(e)
		}
		for _, entry := range entries {
			if entry.Name() != "update.zip" && entry.Name() != "payload" {
				t.Fatal("archive escaped destination")
			}
		}
	})
}
