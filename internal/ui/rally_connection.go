package ui

import (
	"errors"
	"net/http"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/rally"
)

func rallyCredentialError(err error) bool {
	var api *rally.APIError
	return errors.As(err, &api) && (api.Status == http.StatusUnauthorized || api.Status == http.StatusForbidden)
}

func rallyErrorMessage(err error) string {
	if err == nil {
		return ""
	}
	if rallyCredentialError(err) {
		return err.Error() + "\nUpdate your API token in Settings and check access to this workspace/project."
	}
	return err.Error()
}

func (a *App) openRallySettings() {
	a.openSettings()
	a.settingsPage("Rally")
}

// A disabled control still occupies its normal slot and consumes no action.
func enabledButton(w *desktop.Window, title string, enabled, active bool, p palette) bool {
	if enabled {
		return button(w, title, active, p)
	}
	b, out := w.Custom(w.CustomState())
	if out != nil {
		out.FillRect(b, 3, p.Border)
		out.FillRect(inset(b, 1, 1), 2, p.Window)
		labelAt(out, inset(b, 8, 0), title, w.Master().Style().Font, p.Faint)
	}
	return false
}
