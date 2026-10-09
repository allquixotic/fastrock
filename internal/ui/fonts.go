package ui

import (
	"os"
	"path/filepath"
	"runtime"
	"sync"

	"github.com/aarzilli/nucular/font"
	"golang.org/x/image/font/gofont/goitalic"
	"golang.org/x/image/font/gofont/goregular"
)

var uiRegular, uiItalic = goregular.TTF, goitalic.TTF
var fontsOnce sync.Once

// Slint uses the platform UI font. Read installed fonts once, before creating
// the first window; no font filesystem access occurs in a draw callback.
func initUIFonts() {
	fontsOnce.Do(func() {
		regular, italic := "", ""
		switch runtime.GOOS {
		case "windows":
			root := os.Getenv("WINDIR")
			if root == "" {
				root = `C:\Windows`
			}
			regular = filepath.Join(root, "Fonts", "segoeui.ttf")
			italic = filepath.Join(root, "Fonts", "segoeuii.ttf")
		case "darwin":
			regular = "/System/Library/Fonts/SFNS.ttf"
			italic = "/System/Library/Fonts/SFNSItalic.ttf"
		}
		load := func(path string, fallback []byte) []byte {
			data, err := os.ReadFile(path)
			if err != nil {
				return fallback
			}
			face, err := font.NewFace(data, 13)
			if err != nil {
				return fallback
			}
			_ = face.Face.Close()
			return data
		}
		uiRegular = load(regular, uiRegular)
		uiItalic = load(italic, uiItalic)
	})
}
