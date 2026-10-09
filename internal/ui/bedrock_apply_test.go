//go:build fltk_headless

package ui

import (
	"encoding/json"
	"fmt"
	"image"
	"testing"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
)

func TestV71BedrockApplyClearsSameProviderModel(t *testing.T) {
	for _, endpoint := range []int{0, 1} {
		for _, method := range []int{0, 1, 2, 3} {
			t.Run(fmt.Sprintf("endpoint%d/method%d", endpoint, method), func(t *testing.T) {
				a := settingsFixture(t)
				h := desktop.NewHeadlessHarness(0, image.Pt(800, 600), func(*desktop.Window) {})
				a.window = h.Master()
				if err := a.client.Call(a.ctx, "fixture/allowBedrock", nil, nil); err != nil {
					t.Fatal(err)
				}
				provider := "amazon-bedrock"
				if endpoint == 1 {
					provider = "amazon-bedrock-runtime"
				}
				a.catalog.Config = map[string]any{"model_provider": provider, "model": "old-region-model"}
				f := &bedrockForm{Endpoint: endpoint, Method: method, Region: textEditor(" us-west-2 ", false), Profile: textEditor(" test-profile ", false), APIKey: textEditor("fixture-token", false), AccessID: textEditor("fixture-access", false), Secret: textEditor("fixture-secret", false), Token: textEditor("fixture-session", false)}
				a.applyBedrock(f)
				a.applyBedrock(f)
				drain(t, a, func() bool { return a.restartNote != "" })
				var calls []codex.Message
				if err := a.client.Call(a.ctx, "fixture/providerCalls", nil, &calls); err != nil {
					t.Fatal(err)
				}
				writes, setups, gov, logins := 0, 0, 0, 0
				cleared := false
				for _, call := range calls {
					switch call.Method {
					case "account/bedrock/setup":
						var params map[string]any
						if err := json.Unmarshal(call.Params, &params); err != nil {
							t.Fatal(err)
						}
						if params["region"] != "us-west-2" || method == 1 && params["profile"] != "test-profile" {
							t.Fatal("setup kept identifier whitespace")
						}
						setups++
					case "account/login/start":
						logins++
					case "account/bedrock/checkGovCloudRequirements":
						gov++
					case "config/batchWrite":
						writes++
						var params struct {
							Edits []struct {
								KeyPath string
								Value   json.RawMessage
							}
						}
						if err := json.Unmarshal(call.Params, &params); err != nil {
							t.Fatal(err)
						}
						for _, edit := range params.Edits {
							if edit.KeyPath == "model_providers.amazon-bedrock-runtime.aws.region" && string(edit.Value) != `"us-west-2"` {
								t.Fatal("runtime kept region whitespace")
							}
							if method == 1 && edit.KeyPath == "model_providers.amazon-bedrock-runtime.aws.profile" && string(edit.Value) != `"test-profile"` {
								t.Fatal("runtime kept profile whitespace")
							}
							if edit.KeyPath == "model" && string(edit.Value) == "null" {
								cleared = true
							}
						}
					}
				}
				if writes != 1 || !cleared || gov != 1 || f.Busy || f.Feedback.Failed {
					t.Fatal("Apply did not clear/check exactly once", writes, cleared, gov, f.Feedback)
				}
				if endpoint == 1 && method < 2 && setups != 0 {
					t.Fatal("Runtime credentials invoked Mantle setup")
				}
				if endpoint == 0 && method < 2 && setups != 1 {
					t.Fatal("Mantle setup missing or duplicated")
				}
				if method >= 2 && logins != 1 {
					t.Fatal("credential login missing or duplicated")
				}
			})
		}
	}
}
