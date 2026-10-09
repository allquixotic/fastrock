package ui

import (
	"encoding/json"
	"fmt"
	"strings"
	"unicode"

	"github.com/aarzilli/nucular"
)

type bedrockForm struct {
	Endpoint, Method                         int
	Profile, Region, AccessID, Secret, Token *nucular.TextEditor
}

func (a *App) bedrockSettings(w *nucular.Window, s *settingsView) {
	if s.Bedrock == nil {
		s.Bedrock = &bedrockForm{Profile: textEditor("", false), Region: textEditor("us-east-1", false), AccessID: textEditor("", false), Secret: textEditor("", false), Token: textEditor("", false)}
		s.Bedrock.Secret.PasswordChar = '●'
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
	f.Method = w.ComboSimple([]string{"AWS environment", "AWS profile", "Bedrock API token", "AWS access keys"}, f.Method, 28)
	a.field(w, "AWS region", f.Region, false)
	switch f.Method {
	case 1:
		a.field(w, "AWS profile", f.Profile, false)
	case 2:
		a.field(w, "Bedrock API token", f.Secret, false)
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
			a.report(err)
		} else {
			a.toast = "Inputs are valid. Apply to let Codex verify the credentials."
		}
	}
	if primary(w, "Apply provider", a.p) {
		a.applyBedrock(f)
	}
	w.Row(30).Dynamic(2)
	if w.ButtonText("Check GovCloud requirements") {
		a.inspectRPC("GovCloud requirements", "account/bedrock/checkGovCloudRequirements", map[string]any{})
	}
	if w.ButtonText("Return to OpenAI…") {
		a.confirm("Return to OpenAI?", "Sign out of Bedrock and select OpenAI?", func() {
			a.rpc("account/logout", map[string]any{}, func(_ json.RawMessage) { a.configWrite("model_provider", "openai") })
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
		p["profile"] = text(f.Profile)
		return "account/bedrock/setup", p, nil
	case 2:
		if text(f.Secret) == "" {
			return "", nil, fmt.Errorf("enter a Bedrock API token")
		}
		p["type"] = "amazonBedrock"
		p["apiKey"] = text(f.Secret)
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
	method, params, err := bedrockParams(f)
	if err != nil {
		a.report(err)
		return
	}
	endpoint, credential, profile, region := f.Endpoint, f.Method, text(f.Profile), text(f.Region)
	a.rpc(method, params, func(_ json.RawMessage) {
		setText(f.Secret, "")
		setText(f.Token, "")
		if endpoint == 0 {
			a.toast = "Bedrock configured. Restart Codex to use it."
			a.loadSettingsPage("AWS Bedrock")
			return
		}
		var profileValue any
		if credential == 1 {
			profileValue = profile
		}
		a.rpc("config/batchWrite", map[string]any{"reloadUserConfig": true, "edits": []map[string]any{
			{"keyPath": "model_provider", "value": "amazon-bedrock-runtime", "mergeStrategy": "replace"},
			{"keyPath": "model_providers.amazon-bedrock-runtime.aws.profile", "value": profileValue, "mergeStrategy": "replace"},
			{"keyPath": "model_providers.amazon-bedrock-runtime.aws.region", "value": region, "mergeStrategy": "replace"},
		}}, func(_ json.RawMessage) { a.toast = "Bedrock Runtime configured. Restart Codex to use it." })
	})
}
