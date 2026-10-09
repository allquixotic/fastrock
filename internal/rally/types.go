package rally

import (
	"encoding/json"
	"fmt"
	"strconv"
)

// Object retains every WSAPI field, including c_* custom fields and collections.
type Object map[string]any

func (o Object) String(k string) string {
	switch v := o[k].(type) {
	case string:
		return v
	case bool:
		return strconv.FormatBool(v)
	case json.Number:
		return string(v)
	case float64:
		return strconv.FormatFloat(v, 'f', -1, 64)
	case map[string]any:
		if s, ok := v["_refObjectName"].(string); ok {
			return s
		}
		if s, ok := v["Name"].(string); ok {
			return s
		}
		if s, ok := v["_ref"].(string); ok {
			return s
		}
	}
	return ""
}
func (o Object) Ref(k string) string {
	if v, ok := o[k].(map[string]any); ok {
		s, _ := v["_ref"].(string)
		return s
	}
	return o.String(k)
}
func (o Object) Number(k string) float64 {
	switch v := o[k].(type) {
	case float64:
		return v
	case json.Number:
		n, _ := v.Float64()
		return n
	case string:
		n, _ := strconv.ParseFloat(v, 64)
		return n
	}
	return 0
}
func (o Object) Bool(k string) bool { b, _ := o[k].(bool); return b }
func (o Object) ID() string {
	if s := o.String("FormattedID"); s != "" {
		return s
	}
	return o.String("ObjectID")
}
func (o Object) Kind() string {
	s := o.String("_type")
	if s == "" {
		s = "HierarchicalRequirement"
	}
	return s
}
func (o Object) Clone() Object {
	out := make(Object, len(o))
	for k, v := range o {
		out[k] = cloneValue(v)
	}
	return out
}
func cloneValue(v any) any {
	switch x := v.(type) {
	case Object:
		return x.Clone()
	case map[string]any:
		out := make(map[string]any, len(x))
		for k, v := range x {
			out[k] = cloneValue(v)
		}
		return out
	case []any:
		out := make([]any, len(x))
		for i, v := range x {
			out[i] = cloneValue(v)
		}
		return out
	default:
		return v
	}
}

func (o Object) Count(k string) int {
	if v, ok := o[k].(map[string]any); ok {
		return int(Object(v).Number("Count"))
	}
	if a, ok := o[k].([]any); ok {
		return len(a)
	}
	return 0
}

type Query struct {
	Expression string
	Fetch      string
	Order      string
	Workspace  string
	Project    string
	Parents    bool
	Children   bool
	Start      int
	PageSize   int
}
type Page struct {
	Results  []Object
	Total    int
	Start    int
	PageSize int
}
type APIError struct {
	Status   int
	Messages []string
}

func (e *APIError) Error() string {
	return fmt.Sprintf("Rally request failed (%d): %v", e.Status, e.Messages)
}

type Field struct {
	Name          string
	DisplayName   string
	AttributeType string
	Required      bool
	ReadOnly      bool
	AllowedValues []string
}

var ArtifactKinds = []string{"HierarchicalRequirement", "Defect", "Task", "PortfolioItem/Feature", "PortfolioItem/Epic", "TestCase", "TestSet", "DefectSuite"}

func CanonicalKind(s string) (string, bool) {
	aliases := map[string]string{"story": "HierarchicalRequirement", "userstory": "HierarchicalRequirement", "feature": "PortfolioItem/Feature", "epic": "PortfolioItem/Epic", "initiative": "PortfolioItem/Epic"}
	if v, ok := aliases[lower(s)]; ok {
		return v, true
	}
	for _, v := range append(append([]string{}, ArtifactKinds...), "Workspace", "Project", "Iteration", "Release", "User", "ConversationPost", "Attachment", "AttachmentContent", "Revision", "RevisionHistory", "State", "Tag", "TypeDefinition", "AllowedAttributeValue", "TestFolder", "TestCaseResult", "PreliminaryEstimate", "Milestone") {
		if lower(s) == lower(v) {
			return v, true
		}
	}
	return "", false
}
