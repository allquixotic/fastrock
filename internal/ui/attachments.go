package ui

import (
	"fmt"
	"image"
	_ "image/gif"
	_ "image/jpeg"
	_ "image/png"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	_ "golang.org/x/image/webp"
)

const maxAttachmentBytes = 25 << 20

// Called on a worker. File mentions and image payloads have different protocol
// representations; a file extension alone cannot identify a valid image.
func attachmentInputs(paths []string) ([]map[string]any, error) {
	var inputs []map[string]any
	for _, path := range paths {
		path, err := filepath.Abs(path)
		if err != nil {
			return nil, err
		}
		f, err := os.Open(path)
		if err != nil {
			return nil, fmt.Errorf("attachment %s: %w", filepath.Base(path), err)
		}
		info, err := f.Stat()
		if err != nil || !info.Mode().IsRegular() || info.Size() > maxAttachmentBytes {
			f.Close()
			return nil, fmt.Errorf("attachment %s must be a readable regular file of at most 25 MiB", filepath.Base(path))
		}
		prefix := make([]byte, 512)
		n, err := f.Read(prefix)
		if err != nil && err != io.EOF {
			f.Close()
			return nil, err
		}
		mime := http.DetectContentType(prefix[:n])
		if strings.HasPrefix(mime, "image/") {
			_, err = f.Seek(0, io.SeekStart)
			var config image.Config
			if err == nil {
				config, _, err = image.DecodeConfig(f)
			}
			f.Close()
			if err != nil || config.Width <= 0 || config.Height <= 0 || int64(config.Width)*int64(config.Height) > 50_000_000 {
				return nil, fmt.Errorf("attachment %s is an unsupported or oversized image", filepath.Base(path))
			}
			inputs = append(inputs, map[string]any{"type": "localImage", "path": path})
		} else {
			f.Close()
			switch strings.ToLower(filepath.Ext(path)) {
			case ".png", ".jpg", ".jpeg", ".gif", ".webp":
				return nil, fmt.Errorf("attachment %s does not contain a valid image", filepath.Base(path))
			}
			inputs = append(inputs, map[string]any{"type": "text", "text": "@" + strconv.Quote(path)})
		}
	}
	return inputs, nil
}
