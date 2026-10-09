package platform

import (
	"fmt"
	"golang.org/x/mobile/event/key"
	"net/url"
	"strings"

	"runtime"
)

func PrimaryModifier() key.Modifiers {
	if runtime.GOOS == "darwin" {
		return key.ModMeta
	}
	return key.ModControl
}
func OpenURL(target string) error {
	u, e := url.Parse(target)
	if e != nil || strings.ContainsAny(target, "\r\n") || u.Scheme != "https" && u.Scheme != "http" && !(u.Scheme == "mailto" && u.Opaque != "") {
		return fmt.Errorf("invalid web URL")
	}
	switch runtime.GOOS {
	case "darwin":
		return Command("open", target).Run()
	case "windows":
		return Command("rundll32.exe", "url.dll,FileProtocolHandler", target).Run()
	default:
		return Command("xdg-open", target).Run()
	}
}
