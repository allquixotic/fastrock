package platform

import (
	"context"
	"golang.org/x/sys/windows/registry"
)

func SystemLightTheme(context.Context) (bool, error) {
	k, err := registry.OpenKey(registry.CURRENT_USER, `Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`, registry.QUERY_VALUE)
	if err != nil {
		return false, err
	}
	defer k.Close()
	value, _, err := k.GetIntegerValue("AppsUseLightTheme")
	return value != 0, err
}
