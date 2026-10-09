package settings

import (
	"bytes"
	"encoding/json"
	"fmt"
)

type Patch map[string]json.RawMessage

// Diff sends only fields changed by this window. Unrelated changes made in
// another window are therefore never reverted by an old full snapshot.
func Diff(before, after Preferences) Patch {
	a, _ := json.Marshal(before)
	b, _ := json.Marshal(after)
	var old, next map[string]json.RawMessage
	_ = json.Unmarshal(a, &old)
	_ = json.Unmarshal(b, &next)
	patch := Patch{}
	for key, value := range next {
		if !bytes.Equal(old[key], value) {
			patch[key] = value
		}
	}
	for key := range old {
		if _, ok := next[key]; !ok {
			patch[key] = json.RawMessage("null")
		}
	}
	return patch
}

func Apply(p Preferences, patch Patch) (Preferences, error) {
	data, _ := json.Marshal(p)
	var fields map[string]json.RawMessage
	_ = json.Unmarshal(data, &fields)
	// Include omitted optional fields in the allowed set.
	var all map[string]json.RawMessage
	_ = json.Unmarshal([]byte(`{"recentFolders":null,"keymap":null,"busyInput":null,"rallyWorkspace":null,"rallyProject":null,"workingDirectory":null,"views":null,"rallyDisplay":null}`), &all)
	for key := range fields {
		all[key] = nil
	}
	for key, value := range patch {
		if _, ok := all[key]; !ok {
			return p, fmt.Errorf("unknown preference %q", key)
		}
		fields[key] = value
	}
	data, err := json.Marshal(fields)
	if err != nil {
		return p, err
	}
	var next Preferences
	if err = json.Unmarshal(data, &next); err != nil {
		return p, err
	}
	return Normalize(next)
}
