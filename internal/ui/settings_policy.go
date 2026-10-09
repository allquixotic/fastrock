package ui

import (
	"context"
	"slices"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
)

// Sign-in must work even when model discovery fails because there is no
// authenticated account yet. Read its policy independently of the catalog.
func (a *App) refreshSignInPolicy() {
	if a.client == nil || a.serverPaused || a.policyLoading && a.policyServer == a.serverGeneration {
		return
	}
	a.policyRequest++
	a.policyLoading, a.policyServer = true, a.serverGeneration
	request, generation, client, cwd := a.policyRequest, a.serverGeneration, a.client, a.prefs.WorkingDirectory
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 15*time.Second)
		defer cancel()
		var requirements struct{ Requirements *codex.ConfigRequirements }
		err := client.Call(ctx, "configRequirements/read", map[string]any{}, &requirements)
		var config struct{ Config map[string]any }
		if err == nil {
			err = client.Call(ctx, "config/read", map[string]any{"cwd": cwd, "includeLayers": false}, &config)
		}
		a.post(func() {
			if request != a.policyRequest {
				return
			}
			a.policyLoading = false
			if generation != a.serverGeneration || client != a.client {
				return
			}
			a.policyError = ""
			if err != nil {
				a.policyError = err.Error()
				a.catalog.PolicyLoaded = false
				return
			}
			a.catalog.Policy = codex.ConfigRequirements{}
			if requirements.Requirements != nil {
				a.catalog.Policy = *requirements.Requirements
			}
			a.catalog.Requirements = a.catalog.Policy.Features
			a.catalog.PolicyLoaded = true
			if config.Config != nil {
				a.catalog.Config = config.Config
				a.observeProvider(str(config.Config, "model_provider"))
			}
		})
	}, func() {
		if request == a.policyRequest {
			a.policyLoading = false
			a.policyError = errWorkQueueFull.Error()
		}
	})
}

func configChoices(f *configField, allowed []string, constrained bool) (labels, values []string) {
	labels = []string{"Default"}
	for _, value := range f.Spec.Choices {
		if !constrained || slices.Contains(allowed, value) {
			labels = append(labels, value)
			values = append(values, value)
		}
	}
	if constrained {
		for _, value := range allowed {
			if !slices.Contains(values, value) {
				labels = append(labels, value)
				values = append(values, value)
			}
		}
	}
	return labels, values
}

func policyControlledKey(key string) bool {
	return key == "approval_policy" || key == "sandbox_mode" || key == "web_search" || key == "model_provider"
}
