package codex

import (
	"encoding/json"
	"testing"
)

func TestV31ManagedRequirementsPreserveEmptyAndGranular(t *testing.T) {
	var r ConfigRequirements
	if err := json.Unmarshal([]byte(`{"allowedLoginMethods":[],"allowedSandboxModes":["read-only"],"allowedApprovalPolicies":["on-request",{"granular":{"sandbox_approval":true}}],"modelProvider":"managed"}`), &r); err != nil {
		t.Fatal(err)
	}
	if r.LoginAllowed("chatgpt", "") || r.LoginAllowed("apiKey", "") {
		t.Fatal("empty allowed methods became unrestricted")
	}
	if !r.Allows("sandbox_mode", "read-only") || r.Allows("sandbox_mode", "danger-full-access") || !r.Allows("sandbox_mode", nil) {
		t.Fatal("managed sandbox/reset requirements lost")
	}
	if !r.Allows("approval_policy", map[string]any{"granular": map[string]any{"sandbox_approval": true}}) || r.Allows("approval_policy", "never") {
		t.Fatal("granular requirements changed semantics")
	}
	if r.Allows("model_provider", "openai") || !r.Allows("model_provider", "managed") {
		t.Fatal("required provider ignored")
	}
	r = ConfigRequirements{}
	if !r.LoginAllowed("chatgptDeviceCode", "") || !r.LoginAllowed("apiKey", "") || r.LoginAllowed("apiKey", "chatgpt") || r.LoginAllowed("chatgpt", "api") {
		t.Fatal("omitted requirements/forced method semantics wrong")
	}
}
