package codex

import (
	"encoding/json"
	"slices"
)

// Nil lists mean the server did not impose a restriction. An explicitly empty
// list permits no value; preserve that distinction when decoding the protocol.
type ConfigRequirements struct {
	ModelProvider           *string           `json:"modelProvider"`
	AllowedLoginMethods     []string          `json:"allowedLoginMethods"`
	AllowedApprovalPolicies []json.RawMessage `json:"allowedApprovalPolicies"`
	AllowedSandboxModes     []string          `json:"allowedSandboxModes"`
	AllowedWebSearchModes   []string          `json:"allowedWebSearchModes"`
	Features                map[string]bool   `json:"featureRequirements"`
}

func (r ConfigRequirements) AllowedValues(key string) ([]string, bool) {
	switch key {
	case "model_provider":
		if r.ModelProvider != nil {
			return []string{*r.ModelProvider}, true
		}
	case "approval_policy":
		if r.AllowedApprovalPolicies != nil {
			values := []string{}
			for _, raw := range r.AllowedApprovalPolicies {
				var value string
				if json.Unmarshal(raw, &value) == nil {
					values = append(values, value)
				}
			}
			return values, true
		}
	case "sandbox_mode":
		return r.AllowedSandboxModes, r.AllowedSandboxModes != nil
	case "web_search":
		return r.AllowedWebSearchModes, r.AllowedWebSearchModes != nil
	}
	return nil, false
}

func (r ConfigRequirements) Allows(key string, value any) bool {
	if value == nil { // Removing a user override reveals the managed/default value.
		return true
	}
	if key == "approval_policy" && r.AllowedApprovalPolicies != nil {
		encoded, err := json.Marshal(value)
		if err != nil {
			return false
		}
		for _, raw := range r.AllowedApprovalPolicies {
			var a, b any
			if json.Unmarshal(raw, &a) == nil && json.Unmarshal(encoded, &b) == nil {
				x, _ := json.Marshal(a)
				y, _ := json.Marshal(b)
				if string(x) == string(y) {
					return true
				}
			}
		}
		return false
	}
	values, constrained := r.AllowedValues(key)
	if !constrained {
		return true
	}
	text, ok := value.(string)
	return ok && slices.Contains(values, text)
}

func (r ConfigRequirements) LoginAllowed(method, forced string) bool {
	if method == "chatgptDeviceCode" {
		method = "chatgpt"
	}
	if method == "apiKey" {
		method = "api"
	}
	if (forced == "chatgpt" || forced == "api") && forced != method {
		return false
	}
	return r.AllowedLoginMethods == nil || slices.Contains(r.AllowedLoginMethods, method)
}
