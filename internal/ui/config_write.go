package ui

import (
	"encoding/json"
	"fmt"
	"strings"
)

type configEditRequest struct {
	Key, Text string
	Value     any
}

// UI-owned FIFO: a later edit uses the version returned by the preceding write.
// Conflicts retain the editor; retrying a write against an unseen base would
// silently overwrite another window's choice.
func (a *App) nextConfigWrite(s *settingsView) {
	if s.ConfigWriting || len(s.ConfigQueue) == 0 {
		return
	}
	request := s.ConfigQueue[0]
	s.ConfigQueue = s.ConfigQueue[1:]
	if !a.catalog.Policy.Allows(request.Key, request.Value) || policyControlledKey(request.Key) && (!a.catalog.PolicyLoaded || a.policyLoading) {
		s.ConfigFeedback[request.Key] = "This value is not permitted by organization policy."
		if s.ConfigFailed == nil {
			s.ConfigFailed = map[string]bool{}
		}
		s.ConfigFailed[request.Key] = true
		a.nextConfigWrite(s)
		return
	}
	s.ConfigWriting = true
	s.LoadGeneration++ // A read started before this write cannot publish its older version.
	s.Busy = false
	s.ConfigFeedback[request.Key] = "Saving…"
	params := map[string]any{"edits": []map[string]any{{"keyPath": request.Key, "value": request.Value, "mergeStrategy": "replace"}}, "reloadUserConfig": true}
	if s.ConfigVersion != "" {
		params["expectedVersion"] = s.ConfigVersion
	}
	finish := func(err error) {
		s.ConfigWriting = false
		if err != nil {
			if s.ConfigFailed == nil {
				s.ConfigFailed = map[string]bool{}
			}
			s.ConfigFailed[request.Key] = true
			s.ConfigFeedback[request.Key] = err.Error() + ". Your input is retained; reload to review the current value before applying again."
		}
		a.nextConfigWrite(s)
	}
	a.rpcInline("config/batchWrite", params, func(raw json.RawMessage) {
		var response struct {
			Version            string
			OverriddenMetadata *struct{ Message string }
		}
		if err := json.Unmarshal(raw, &response); err != nil {
			finish(err)
			return
		}
		if response.Version == "" {
			finish(fmt.Errorf("Codex did not return a configuration version"))
			return
		}
		s.ConfigVersion = response.Version
		if request.Key == "model_provider" || strings.HasPrefix(request.Key, "model_providers.") {
			a.refreshConfiguredProvider()
		}
		s.ConfigFeedback[request.Key] = "Saved"
		delete(s.ConfigFailed, request.Key)
		if response.OverriddenMetadata != nil {
			s.ConfigFeedback[request.Key] = response.OverriddenMetadata.Message
		}
		for i := range s.Fields {
			f := &s.Fields[i]
			if f.Key == request.Key {
				f.Value = request.Value
				f.Baseline = request.Text
			}
		}
		finish(nil)
		if len(s.ConfigQueue) == 0 && !s.ConfigWriting && configurationPage(s.Page) {
			a.loadSettingsPage(s.Page)
		}
	}, finish)
}

func mergeConfigFields(old, fresh []configField) []configField {
	dirty := map[string]configField{}
	for _, f := range old {
		if text(f.Editor) != f.Baseline {
			dirty[f.Key] = f
		}
	}
	for i := range fresh {
		if f, ok := dirty[fresh[i].Key]; ok {
			fresh[i].Editor = f.Editor
		}
	}
	return fresh
}
