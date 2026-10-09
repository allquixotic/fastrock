package ui

import (
	"encoding/json"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
)

func describeAccount(data map[string]any, provider string) (title, detail string, facts []accountFact) {
	facts = append(facts, accountFact{"Model provider", fallback(provider, "openai")})
	account, _ := data["account"].(map[string]any)
	if account == nil {
		if required, _ := data["requiresOpenaiAuth"].(bool); required {
			return "Not signed in", "Sign in with an allowed method to use OpenAI models.", facts
		}
		return "No OpenAI sign-in needed", "The current provider does not need an OpenAI account.", facts
	}
	switch str(account, "type") {
	case "apiKey":
		return "Signed in with an API key", "Usage is billed to your OpenAI Platform account.", facts
	case "chatgpt":
		if email := str(account, "email"); email != "" {
			facts = append(facts, accountFact{"Email", email})
		}
		if plan := str(account, "planType"); plan != "" {
			facts = append(facts, accountFact{"Plan", strings.ReplaceAll(plan, "_", " ")})
		}
		return "Signed in with ChatGPT", "Your ChatGPT account supplies access to Codex.", facts
	case "amazonBedrock":
		detail = "Credentials come from your AWS configuration."
		if managed, _ := account["usesCodexManagedCredentials"].(bool); managed {
			detail = "Codex stores the Bedrock credentials. Signing out removes them."
		}
		return "Using Amazon Bedrock", detail, facts
	default:
		return "Signed in", "Account type: " + str(account, "type"), facts
	}
}

func (a *App) drawAccountCard(w *desktop.Window, id, heading, detail string, facts []accountFact) {
	height := titleHeight(w) + 20 + len(facts)*30
	if detail != "" {
		height += 50
	}
	w.Row(height).Dynamic(1)
	if card := w.GroupBegin(id, desktop.WindowBorder|desktop.WindowNoScrollbar); card != nil {
		title(card, heading, a.p)
		if detail != "" {
			card.Row(42).Dynamic(1)
			card.LabelWrap(detail)
		}
		for _, fact := range facts {
			card.Row(28).Ratio(.38, .62)
			card.Label(fact.Name, "LC")
			card.Label(cut(fact.Value, 300), "LC")
		}
		card.GroupEnd()
	}
}

func (a *App) drawAccount(w *desktop.Window, s *settingsView) {
	heading, detail, facts := describeAccount(a.accountData, str(a.catalog.Config, "model_provider"))
	if !a.accountLoaded {
		heading, detail, facts = "Account unavailable", "Refresh to read your account from Codex.", nil
		if a.accountLoading {
			heading, detail, facts = "Loading account…", "", nil
		}
	}
	a.drawAccountCard(w, "account-summary", heading, detail, facts)
	if a.accountError != "" {
		a.drawSettingsError(w, a.accountError)
	}
	if s.LoginError != "" {
		a.drawSettingsError(w, s.LoginError)
	}
	if s.LoginBusy {
		muted(w, "Finish signing in to continue.", a.p)
		if s.LoginID == "" {
			muted(w, "Starting sign-in…", a.p)
		}
		if s.LoginCode != "" {
			title(w, "Sign-in code: "+s.LoginCode, a.p)
		}
		if s.LoginURL != "" {
			muted(w, cut(s.LoginURL, 256), a.p)
			w.Row(28).Dynamic(2)
			if w.ButtonText("Copy sign-in link") {
				a.copyText(s.LoginURL)
			}
			if w.ButtonText("Open sign-in link") {
				a.openURL(s.LoginURL)
			}
		}
		if s.LoginID != "" {
			w.Row(28).Static(130)
			if w.ButtonText("Cancel sign-in") {
				a.cancelAccountLogin(s)
			}
		}
	} else if a.client != nil && !a.serverPaused && (!a.catalog.PolicyLoaded || a.policyLoading) {
		muted(w, "Sign-in choices are unavailable until Codex policy loads.", a.p)
		if a.policyError != "" {
			a.drawSettingsError(w, a.policyError)
		}
		w.Row(28).Static(150)
		if w.ButtonText("Reload sign-in policy") {
			a.refreshSignInPolicy()
		}
	} else if a.client != nil && !a.serverPaused {
		forced := str(a.catalog.Config, "forced_login_method")
		chatgpt := a.catalog.Policy.LoginAllowed("chatgpt", forced)
		api := a.catalog.Policy.LoginAllowed("api", forced)
		if chatgpt {
			w.Row(30).Dynamic(2)
			if w.ButtonText("Sign in with ChatGPT") {
				a.login(map[string]any{"type": "chatgpt"})
			}
			if w.ButtonText("Device code sign-in") {
				a.login(map[string]any{"type": "chatgptDeviceCode"})
			}
		}
		if api {
			s.Secret.PasswordChar = '●'
			a.field(w, "API key", s.Secret, false)
			w.Row(28).Static(150)
			if w.ButtonText("Sign in with key") && strings.TrimSpace(text(s.Secret)) != "" {
				a.login(map[string]any{"type": "apiKey", "apiKey": text(s.Secret)})
				setText(s.Secret, "")
			}
		}
		if !chatgpt || !api {
			muted(w, "Available sign-in methods are restricted by organization policy.", a.p)
		}
		if account, _ := a.accountData["account"].(map[string]any); account != nil {
			w.Row(28).Static(130)
			if w.ButtonText("Sign out…") {
				a.confirm("Sign out?", "Sign out of the current Codex account?", func() {
					a.settingsFormCall("logout", "account/logout", map[string]any{}, func(json.RawMessage) {
						a.refreshAccount()
						a.refreshUsage()
						a.refreshConfiguredProvider()
					})
				})
			}
		}
	}
	u := &a.accountUsage
	rows := usageRows(u.Data, time.Now())
	detail = ""
	if u.Loading {
		detail = "Refreshing usage…"
	} else if len(rows) == 0 {
		detail = "Usage limits are not available for this account."
	}
	a.drawAccountCard(w, "account-usage", "Usage and limits", detail, rows)
	if u.Error != "" {
		a.drawSettingsError(w, u.Error)
	}
	w.Row(28).Static(150)
	if w.ButtonText("Refresh usage") {
		a.refreshUsage()
	}
}

func (a *App) cancelAccountLogin(s *settingsView) {
	if s.LoginID == "" {
		return
	}
	id := s.LoginID
	a.rpcInline("account/login/cancel", map[string]any{"loginId": id}, func(json.RawMessage) {
		if s.LoginID == id {
			s.LoginGeneration++
			s.LoginBusy = false
			s.LoginID, s.LoginCode, s.LoginURL = "", "", ""
		}
	}, func(err error) { s.LoginError = err.Error() })
}

func (a *App) drawSettingsError(w *desktop.Window, text string) {
	style := w.Master().Style()
	old := style.Text.Color
	style.Text.Color = a.p.Danger
	w.Row(40).Dynamic(1)
	w.LabelWrap(text)
	style.Text.Color = old
}
