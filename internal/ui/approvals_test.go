package ui

import (
	"github.com/allquixotic/fastrock/internal/codex"
	"testing"
)

func TestServerApprovalDecisionsAndPermissions(t *testing.T) {
	p := codex.Decode([]byte(`{"availableDecisions":["accept","decline"]}`))
	choices := approvalChoices("item/commandExecution/requestApproval", p)
	if len(choices) != 2 || choices[1].Key != 'd' {
		t.Fatal(choices)
	}
	p = codex.Decode([]byte(`{"availableDecisions":[{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["go","test"]}}]}`))
	choices = approvalChoices("item/commandExecution/requestApproval", p)
	if len(choices) != 1 || choices[0].Key != 'p' {
		t.Fatal(choices)
	}
	choices = approvalChoices("item/permissions/requestApproval", map[string]any{"permissions": map[string]any{"network": true}})
	if len(choices) != 4 || choices[1].Result.(map[string]any)["strictAutoReview"] != true || choices[2].Result.(map[string]any)["scope"] != "session" {
		t.Fatal(choices)
	}
}
