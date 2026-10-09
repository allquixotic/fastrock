package ui

import (
	"context"
	"github.com/allquixotic/fastrock/internal/platform"
	"time"
)

func (a *App) observeTheme() {
	enabled := false
	var poll <-chan time.Time
	timer := time.NewTimer(time.Hour)
	timer.Stop()
	defer timer.Stop()
	for {
		select {
		case enabled = <-a.themeChanges:
		case <-poll:
		case <-a.ctx.Done():
			return
		}
		timer.Stop()
		poll = nil
		if !enabled {
			continue
		}
		ctx, cancel := context.WithTimeout(a.ctx, 3*time.Second)
		light, err := platform.SystemLightTheme(ctx)
		cancel()
		if err == nil {
			a.post(func() {
				if a.prefs.Theme == "system" && a.systemLight != light {
					a.systemLight = light
					a.applyTheme()
				}
			})
		}
		timer.Reset(5 * time.Second)
		poll = timer.C
	}
}
