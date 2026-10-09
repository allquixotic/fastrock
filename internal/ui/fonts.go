package ui

import (
	"os"
	"runtime"
	"sync"

	"golang.org/x/image/font/gofont/goitalic"
	"golang.org/x/image/font/gofont/gomono"
	"golang.org/x/image/font/gofont/goregular"
)

var uiRegular, uiItalic, uiMono = goregular.TTF, goitalic.TTF, gomono.TTF
var fontsOnce sync.Once

// Slint uses the platform UI font. Read installed fonts once, before creating
// the first window; no font filesystem access occurs in a draw callback.
func initUIFonts() {
	fontsOnce.Do(func() {
		for variant := regularFont; variant <= monoBoldItalicFont; variant++ {
			for _, path := range fontPaths(runtime.GOOS, os.Getenv("WINDIR"), variant) {
				data, err := loadFontFile(path, variant)
				if err != nil {
					continue
				}
				switch variant {
				case regularFont:
					uiRegular = data
				case boldFont:
					uiBold = data
				case italicFont:
					uiItalic = data
				case boldItalicFont:
					uiBoldItalic = data
				case monoFont:
					uiMono = data
				case monoBoldFont:
					uiMonoBold = data
				case monoItalicFont:
					uiMonoItalic = data
				case monoBoldItalicFont:
					uiMonoBoldItalic = data
				}
				break
			}
		}
	})
}
