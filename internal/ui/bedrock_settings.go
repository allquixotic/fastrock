package ui

import (
	"encoding/json"
	"fmt"
	"strings"
	"unicode"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/codex"
)

type bedrockForm struct {
	Generation                                       uint64
	Busy                                             bool
	Feedback                                         settingFeedback
	Endpoint, Method                                 int
	Profile, Region, AccessID, Secret, APIKey, Token *nucular.TextEditor
}

func (a *App) bedrockSettings(w *nucular.Window, s *settingsView) {
	if s.Bedrock == nil {
		s.Bedrock = &bedrockForm{Profile: textEditor("", false), Region: textEditor("us-east-1", false), AccessID: textEditor("", false), Secret: textEditor("", false), APIKey: textEditor("", false), Token: textEditor("", false)}
		s.Bedrock.Secret.PasswordChar = '●'
		s.Bedrock.APIKey.PasswordChar = '●'
		s.Bedrock.Token.PasswordChar = '●'
	}
	f := s.Bedrock
	w.Row(30).Dynamic(2)
	if button(w, "Bedrock Mantle", f.Endpoint == 0, a.p) {
		f.Endpoint = 0
	}
	if button(w, "Bedrock Runtime", f.Endpoint == 1, a.p) {
		f.Endpoint = 1
	}
	w.Row(30).Dynamic(1)
	method := w.ComboSimple([]string{"AWS environment", "AWS profile", "Bedrock API token", "AWS access keys"}, f.Method, 28)
	if method != f.Method {
		setText(f.Secret, "")
		setText(f.APIKey, "")
		setText(f.Token, "")
		f.Method = method
	}
	title(w, "AWS region", a.p)
	regions, selected := bedrockRegionChoices(text(f.Region))
	w.Row(30).Dynamic(1)
	if chosen := w.ComboSimple(regions, selected, 28); chosen != selected && chosen >= 0 && chosen < len(bedrockRegions) {
		setText(f.Region, bedrockRegions[chosen].code)
	}
	if strings.HasPrefix(text(f.Region), "us-gov-") {
		muted(w, "GovCloud requires an eligible organization and supported credentials.", a.p)
	}
	switch f.Method {
	case 1:
		a.field(w, "AWS profile", f.Profile, false)
	case 2:
		a.field(w, "Bedrock API token", f.APIKey, false)
	case 3:
		a.field(w, "Access key ID", f.AccessID, false)
		a.field(w, "Secret access key", f.Secret, false)
		a.field(w, "Session token (optional)", f.Token, false)
	}
	w.Row(30).Dynamic(3)
	if w.ButtonText("Discover credentials") {
		a.loadSettingsPage("AWS Bedrock")
	}
	if w.ButtonText("Check inputs") {
		_, _, err := bedrockParams(f)
		if err != nil {
			f.Feedback = settingFeedback{Failed: true, Message: err.Error()}
		} else {
			f.Feedback = settingFeedback{Message: "Inputs are valid. Apply to let Codex verify the credentials."}
		}
	}
	if !f.Busy && primary(w, "Apply provider", a.p) {
		a.applyBedrock(f)
	}
	if f.Busy {
		muted(w, "Applying provider…", a.p)
	}
	a.drawSettingFeedback(w, f.Feedback)
	w.Row(30).Dynamic(2)
	if w.ButtonText("Check GovCloud requirements") {
		a.inspectRPC("GovCloud requirements", "account/bedrock/checkGovCloudRequirements", map[string]any{})
	}
	if w.ButtonText("Return to OpenAI…") {
		a.confirm("Return to OpenAI?", "Select OpenAI and clear Bedrock provider overrides? Existing account credentials are preserved.", func() {
			edits := []map[string]any{{"keyPath": "model_provider", "value": "openai", "mergeStrategy": "replace"}, {"keyPath": "model", "value": nil, "mergeStrategy": "replace"}}
			for _, provider := range []string{"amazon-bedrock", "amazon-bedrock-runtime"} {
				edits = append(edits, map[string]any{"keyPath": "model_providers." + provider + ".aws", "value": nil, "mergeStrategy": "replace"})
			}
			a.settingsFormCall("openai", "config/batchWrite", map[string]any{"edits": edits, "reloadUserConfig": true}, func(json.RawMessage) { a.providerRestartPrompt() })
		})
	}
	for _, item := range s.Items {
		if item.Name == "" {
			continue
		}
		w.Row(28).Dynamic(1)
		if w.ButtonText("Use profile: " + item.Name) {
			setText(f.Profile, item.Name)
			f.Method = 1
			if region := str(item.Raw, "region"); region != "" {
				setText(f.Region, region)
			}
		}
	}
	muted(w, "Codex stores and validates credentials. Restart Codex after changing providers; unsent drafts are retained.", a.p)
}
func bedrockParams(f *bedrockForm) (string, map[string]any, error) {
	region := strings.TrimSpace(text(f.Region))
	if region == "" || strings.IndexFunc(region, func(r rune) bool { return !(unicode.IsLower(r) || unicode.IsDigit(r) || r == '-') }) >= 0 {
		return "", nil, fmt.Errorf("enter an AWS region")
	}
	p := map[string]any{"region": region}
	switch f.Method {
	case 0:
		p["type"] = "environment"
		return "account/bedrock/setup", p, nil
	case 1:
		if strings.TrimSpace(text(f.Profile)) == "" {
			return "", nil, fmt.Errorf("enter an AWS profile")
		}
		p["type"] = "profile"
		p["profile"] = strings.TrimSpace(text(f.Profile))
		return "account/bedrock/setup", p, nil
	case 2:
		if text(f.APIKey) == "" {
			return "", nil, fmt.Errorf("enter a Bedrock API token")
		}
		p["type"] = "amazonBedrock"
		p["apiKey"] = text(f.APIKey)
	case 3:
		if text(f.AccessID) == "" || text(f.Secret) == "" {
			return "", nil, fmt.Errorf("enter both AWS access key fields")
		}
		p["type"] = "amazonBedrockAccessKeys"
		p["accessKeyId"] = text(f.AccessID)
		p["secretAccessKey"] = text(f.Secret)
		if text(f.Token) != "" {
			p["sessionToken"] = text(f.Token)
		}
	default:
		return "", nil, fmt.Errorf("choose a credential method")
	}
	return "account/login/start", p, nil
}
func (a *App) applyBedrock(f *bedrockForm) {
	if f.Busy {
		return
	}
	method, params, err := bedrockParams(f)
	if err != nil {
		f.Feedback = settingFeedback{Failed: true, Message: err.Error()}
		return
	}
	f.Generation++
	generation, client, server := f.Generation, a.client, a.serverGeneration
	current := func() bool { return generation == f.Generation && client == a.client && server == a.serverGeneration }
	endpoint, credential, profile, region := f.Endpoint, f.Method, strings.TrimSpace(text(f.Profile)), strings.TrimSpace(text(f.Region))
	secret, apiKey, token := text(f.Secret), text(f.APIKey), text(f.Token)
	f.Busy = true
	f.Feedback = settingFeedback{Pending: true, Message: "Applying provider…"}
	failed := func(err error) {
		if generation == f.Generation {
			f.Busy = false
			f.Feedback = settingFeedback{Failed: true, Message: err.Error()}
		}
	}
	finish := func() {
		if !current() {
			return
		}
		f.Busy = false
		f.Feedback = settingFeedback{Message: "Provider saved. Restart Codex to apply it."}
		if text(f.Secret) == secret {
			setText(f.Secret, "")
		}
		if text(f.APIKey) == apiKey {
			setText(f.APIKey, "")
		}
		if text(f.Token) == token {
			setText(f.Token, "")
		}
		a.rpcInline("account/bedrock/checkGovCloudRequirements", map[string]any{}, func(raw json.RawMessage) {
			if !current() {
				return
			}
			result := codex.Decode(raw)
			if yes, _ := result["isGovCloud"].(bool); yes {
				a.confirm("AWS GovCloud guidance", "Read the AWS GovCloud application and network guidance before continuing. Your organization may require API-only sign-in and restricted endpoint access.", a.providerRestartPrompt)
			} else {
				a.providerRestartPrompt()
			}
		}, func(err error) {
			if !current() {
				return
			}
			f.Feedback = settingFeedback{Failed: true, Message: "Provider saved; GovCloud check failed: " + err.Error() + ". Restart Codex when ready."}
			a.restartNote = "Provider saved. Restart Codex to apply it."
		})
	}
	write := func() {
		if !current() {
			return
		}
		provider := "amazon-bedrock"
		if endpoint == 1 {
			provider = "amazon-bedrock-runtime"
		}
		edits := []map[string]any{}
		if endpoint == 1 {
			var profileValue any
			if credential == 1 {
				profileValue = profile
			}
			edits = append(edits, map[string]any{"keyPath": "model_provider", "value": provider, "mergeStrategy": "replace"}, map[string]any{"keyPath": "model_providers." + provider + ".aws.profile", "value": profileValue, "mergeStrategy": "replace"}, map[string]any{"keyPath": "model_providers." + provider + ".aws.region", "value": region, "mergeStrategy": "replace"})
		}
		// The same provider can have different models after a region or
		// credential change. Let its next catalog choose a compatible default.
		edits = append(edits, map[string]any{"keyPath": "model", "value": nil, "mergeStrategy": "replace"})
		a.rpcInline("config/batchWrite", map[string]any{"edits": edits, "reloadUserConfig": true}, func(json.RawMessage) { finish() }, failed)
	}
	// Runtime environment/profile credentials do not need Mantle setup: that RPC
	// changes the provider and can persist credentials for the wrong endpoint.
	if endpoint == 1 && credential < 2 {
		write()
		return
	}
	a.rpcInline(method, params, func(json.RawMessage) { write() }, failed)
}
func (a *App) providerRestartPrompt() {
	a.restartNote = "Provider saved. Restart Codex to apply it."
	a.refreshConfiguredProvider()
	a.confirm("Restart Codex now?", "The provider is saved. Restarting stops running turns in all windows; drafts stay open. Cancel to restart later.", a.restartServer)
}
