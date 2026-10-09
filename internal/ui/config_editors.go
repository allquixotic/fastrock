package ui

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"

	"github.com/aarzilli/nucular"
	"github.com/pelletier/go-toml/v2"
)

func configLeaf(key string) string {
	var parsed map[string]any
	if toml.Unmarshal([]byte(key+" = 0"), &parsed) != nil {
		return key
	}
	for len(parsed) == 1 {
		for name, value := range parsed {
			if child, ok := value.(map[string]any); ok {
				parsed = child
			} else {
				return name
			}
			break
		}
	}
	return key
}
func configTableText(key string, value any) string {
	if value == nil {
		value = map[string]any{}
	}
	encoded, err := toml.Marshal(map[string]any{configLeaf(key): value})
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(encoded))
}
func hasRedacted(value any) bool {
	switch v := value.(type) {
	case string:
		return v == "[redacted]"
	case map[string]any:
		for _, child := range v {
			if hasRedacted(child) {
				return true
			}
		}
	case []any:
		for _, child := range v {
			if hasRedacted(child) {
				return true
			}
		}
	}
	return false
}
func prepareConfigField(f *configField) {
	switch {
	case f.Key == "notify":
		f.Kind = "words"
		var words []string
		switch values := f.Value.(type) {
		case []any:
			for _, v := range values {
				if s, ok := v.(string); ok {
					words = append(words, s)
				}
			}
		case []string:
			words = values
		}
		setText(f.Editor, displayCommand(words))
	case f.Spec != nil && f.Spec.Type == "object" || object(f.Value) != nil:
		f.Kind = "toml"
		f.Editor.Flags = nucular.EditBox | nucular.EditSoftWrap | nucular.EditNoHorizontalScroll
		setText(f.Editor, configTableText(f.Key, f.Value))
	}
	f.Protected = hasRedacted(f.Value)
	f.Baseline = text(f.Editor)
}
func configGroup(key string) string {
	root, _, _ := strings.Cut(key, ".")
	switch root {
	case "model", "model_provider", "model_providers", "model_reasoning_effort", "model_reasoning_summary", "model_verbosity", "model_context_window", "model_auto_compact_token_limit", "model_instructions_file", "service_tier", "personality":
		return "Model and responses"
	case "approval_policy", "sandbox_mode", "sandbox_workspace_write", "permissions", "windows", "web_search", "tools":
		return "Permissions and tools"
	case "features", "experimental", "mcp_servers", "plugins", "skills", "hooks":
		return "Features and extensions"
	case "memories", "history", "session", "sessions_dir":
		return "Memory and history"
	case "shell_environment_policy", "shell_tool", "shell_command", "exec", "terminal":
		return "Shell and environment"
	default:
		return "General"
	}
}
func configGroupsSorted(fields []configField) {
	sort.SliceStable(fields, func(i, j int) bool {
		a, b := configGroup(fields[i].Key), configGroup(fields[j].Key)
		if a != b {
			return a < b
		}
		return fields[i].Key < fields[j].Key
	})
}
func tableFieldValue(f *configField) (any, error) {
	if strings.TrimSpace(text(f.Editor)) == "" {
		return nil, nil
	}
	var doc map[string]any
	if err := toml.Unmarshal([]byte(text(f.Editor)), &doc); err != nil {
		return nil, fmt.Errorf("invalid TOML: %w", err)
	}
	leaf := configLeaf(f.Key)
	value, ok := doc[leaf]
	if !ok || len(doc) != 1 {
		return nil, fmt.Errorf("use only the %s table in this snippet", leaf)
	}
	if object(value) == nil {
		return nil, fmt.Errorf("%s requires a TOML table", leaf)
	}
	return value, nil
}

// Keep a JSON representation for unknown scalar/array fields; known tables and
// commands use editors that round-trip the native configuration syntax.
func configValueText(value any) string { encoded, _ := json.Marshal(value); return string(encoded) }

func typedConfigFields(config map[string]any, specs map[string]*configSpec) []configField {
	var fields []configField
	var visit func(string, any)
	visit = func(key string, value any) {
		spec := specs[key]
		descendants := false
		if spec == nil {
			for name := range specs {
				if strings.HasPrefix(name, key+".") {
					descendants = true
					break
				}
			}
		}
		if m, ok := value.(map[string]any); ok && (key == "" || spec != nil && spec.Type != "object" || descendants) {
			for name, child := range m {
				path := configKey(name)
				if key != "" {
					path = key + "." + path
				}
				visit(path, child)
			}
			return
		}
		if key == "" || value == "[redacted]" {
			return
		}
		kind, display := "json", configValueText(value)
		switch v := value.(type) {
		case string:
			kind, display = "string", v
		case bool:
			kind = "bool"
		}
		field := configField{Key: key, Kind: kind, Value: value, Spec: spec, Editor: textEditor(display, false), Search: strings.ToLower(key)}
		if spec != nil {
			field.Search += " " + strings.ToLower(spec.Description)
		}
		prepareConfigField(&field)
		fields = append(fields, field)
	}
	visit("", config)
	configGroupsSorted(fields)
	return fields
}
