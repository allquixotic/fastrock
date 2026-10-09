package ui

import "testing"

func TestConfigurationFieldsRetainTypes(t *testing.T) {
	f := configFields(map[string]any{"model": "gpt-6.1-sol", "features": map[string]any{"fast_mode": false}, "count": float64(4)})
	if len(f) != 3 || f[1].Key != "features.fast_mode" || f[1].Kind != "bool" {
		t.Fatalf("%+v", f)
	}
}

func TestConfigurationQuotesLiteralDots(t *testing.T) {
	fields := configFields(map[string]any{"mcp_servers": map[string]any{"my.server": map[string]any{"enabled": true}}})
	if len(fields) != 1 || fields[0].Key != `mcp_servers."my.server".enabled` {
		t.Fatalf("%+v", fields)
	}
}

func TestConfigurationOriginLocksHigherPrecedence(t *testing.T) {
	data := map[string]any{"origins": map[string]any{"model": map[string]any{"name": map[string]any{"type": "project"}}}}
	if origin, locked := configOrigin(data, "model"); origin != "project" || !locked {
		t.Fatalf("%s %v", origin, locked)
	}
}

func TestSchemaIncludesUnsetOptionsAndRuntimeKeys(t *testing.T) {
	fields := schemaConfigFields(map[string]any{"future_setting": "custom"})
	model, personality, future := false, false, false
	for _, f := range fields {
		switch f.Key {
		case "model":
			model = f.Spec != nil && f.Spec.Description != ""
		case "personality":
			personality = f.Spec != nil && len(f.Spec.Options) > 1
		case "future_setting":
			future = f.Kind == "string" && text(f.Editor) == "custom"
		}
	}
	if !model || !personality || !future {
		t.Fatalf("model=%v personality=%v future=%v", model, personality, future)
	}
}
