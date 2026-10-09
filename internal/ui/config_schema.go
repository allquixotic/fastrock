package ui

import (
	_ "embed"
	"encoding/json"
	"fmt"
	"math"
	"strings"
	"sync"
)

// Snapshot of Codex's Apache-2.0 config schema; see THIRD_PARTY_NOTICES.md.
// Runtime config values and unknown fields still come from the installed CLI.
//
//go:embed config_schema.json
var configSchemaJSON []byte

type configSpec struct {
	Description, Type string
	Default           any
	Choices           []string
	Options           []string
	Minimum, Maximum  *float64
}

var configSpecs = sync.OnceValue(func() map[string]*configSpec {
	var root map[string]any
	_ = json.Unmarshal(configSchemaJSON, &root)
	definitions, _ := root["definitions"].(map[string]any)
	var resolve func(map[string]any, int) map[string]any
	resolve = func(node map[string]any, depth int) map[string]any {
		if depth > 12 {
			return node
		}
		result := map[string]any{}
		if ref := str(node, "$ref"); strings.HasPrefix(ref, "#/definitions/") {
			base, _ := definitions[strings.TrimPrefix(ref, "#/definitions/")].(map[string]any)
			for k, v := range resolve(base, depth+1) {
				result[k] = v
			}
		}
		if all, ok := node["allOf"].([]any); ok {
			for _, part := range all {
				if p, ok := part.(map[string]any); ok {
					for k, v := range resolve(p, depth+1) {
						result[k] = v
					}
				}
			}
		}
		for k, v := range node {
			result[k] = v
		}
		return result
	}
	result := map[string]*configSpec{}
	var collect func(map[string]any, string, int)
	collect = func(node map[string]any, path string, depth int) {
		node = resolve(node, 0)
		properties, _ := node["properties"].(map[string]any)
		if len(properties) > 0 && depth < 3 {
			for name, value := range properties {
				if path == "" && (name == "profile" || name == "profiles" || name == "tui" || name == "notice" || name == "projects") {
					continue
				}
				child, _ := value.(map[string]any)
				key := configKey(name)
				if path != "" {
					key = path + "." + key
				}
				collect(child, key, depth+1)
			}
			return
		}
		if path == "" {
			return
		}
		spec := &configSpec{Description: str(node, "description"), Type: str(node, "type"), Default: node["default"]}
		if n, ok := node["minimum"].(float64); ok {
			spec.Minimum = &n
		}
		if n, ok := node["maximum"].(float64); ok {
			spec.Maximum = &n
		}
		if choices, ok := node["enum"].([]any); ok {
			for _, v := range choices {
				if s, ok := v.(string); ok {
					spec.Choices = append(spec.Choices, s)
				}
			}
		}
		if variants, ok := node["oneOf"].([]any); ok {
			var choices []string
			for _, v := range variants {
				m, _ := v.(map[string]any)
				options, _ := m["enum"].([]any)
				if len(options) != 1 {
					choices = nil
					break
				}
				s, ok := options[0].(string)
				if !ok {
					choices = nil
					break
				}
				choices = append(choices, s)
			}
			if len(choices) > 0 {
				spec.Choices = choices
				spec.Type = "string"
			}
		}
		result[path] = spec
		if len(spec.Choices) > 0 {
			spec.Options = append([]string{"Default"}, spec.Choices...)
		}
	}
	collect(root, "", 0)
	return result
})

// Called by the settings worker, never by a draw callback.
func schemaConfigFields(config map[string]any) []configField {
	specs := configSpecs()
	fields := typedConfigFields(config, specs)
	seen := map[string]bool{}
	for i := range fields {
		fields[i].Spec = specs[fields[i].Key]
		fields[i].Search = strings.ToLower(fields[i].Key)
		if fields[i].Spec != nil {
			fields[i].Search += " " + strings.ToLower(fields[i].Spec.Description)
		}
		seen[fields[i].Key] = true
	}
	for key, spec := range specs {
		if seen[key] {
			continue
		}
		f := configField{Key: key, Spec: spec, Kind: "json", Value: spec.Default, Editor: textEditor("", false)}
		f.Search = strings.ToLower(key + " " + spec.Description)
		if spec.Type == "string" {
			f.Kind = "string"
		}
		if spec.Type == "boolean" {
			f.Kind = "bool"
		}
		if spec.Default != nil {
			if s, ok := spec.Default.(string); ok {
				setText(f.Editor, s)
			} else {
				b, _ := json.Marshal(spec.Default)
				setText(f.Editor, string(b))
			}
		}
		prepareConfigField(&f)
		fields = append(fields, f)
	}
	configGroupsSorted(fields)
	return fields
}

func configFieldValue(f *configField) (any, error) {
	var value any
	if f.Protected {
		return nil, fmt.Errorf("this table contains protected values; edit config.toml directly")
	}
	if strings.TrimSpace(text(f.Editor)) == "" {
		return nil, nil
	}
	if f.Kind == "words" {
		return splitSettingWords(text(f.Editor))
	}
	if f.Kind == "toml" {
		return tableFieldValue(f)
	}
	if f.Kind == "string" {
		value = text(f.Editor)
		if value == "" {
			return nil, nil
		}
	} else if err := json.Unmarshal([]byte(text(f.Editor)), &value); err != nil {
		return nil, err
	}
	if f.Spec != nil {
		if n, ok := value.(float64); ok {
			if f.Spec.Type == "integer" && n != math.Trunc(n) {
				return nil, fmt.Errorf("%s requires a whole number", f.Key)
			}
			if f.Spec.Minimum != nil && n < *f.Spec.Minimum {
				return nil, fmt.Errorf("%s must be at least %g", f.Key, *f.Spec.Minimum)
			}
			if f.Spec.Maximum != nil && n > *f.Spec.Maximum {
				return nil, fmt.Errorf("%s must be at most %g", f.Key, *f.Spec.Maximum)
			}
		}
	}
	return value, nil
}
