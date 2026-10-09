//go:build (!windows && !darwin && !linux && !freebsd) || android

package clipboard

import "errors"

func Start() {
}

func Get() string {
	return ""
}

func GetPrimary() string {
	return ""
}

func Set(text string) {
}

func Read(bool) (string, error) { return "", errors.New("clipboard is unavailable on this platform") }
func Write(string) error        { return errors.New("clipboard is unavailable on this platform") }
