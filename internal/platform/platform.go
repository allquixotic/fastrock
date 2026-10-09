package platform

import (
	"fmt"
	"golang.org/x/mobile/event/key"
	"net/url"
	"os/exec"
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
	if e != nil || u.Scheme != "https" && u.Scheme != "http" {
		return fmt.Errorf("invalid web URL")
	}
	switch runtime.GOOS {
	case "darwin":
		return exec.Command("open", target).Run()
	case "windows":
		return exec.Command("rundll32.exe", "url.dll,FileProtocolHandler", target).Run()
	default:
		return exec.Command("xdg-open", target).Run()
	}
}
