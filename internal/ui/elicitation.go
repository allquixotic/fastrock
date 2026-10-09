package ui

import (
	"fmt"
	"math"
	"net/url"
	"strconv"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/aarzilli/nucular"
)

// The app-server's MCP schema is a closed set of primitives and string enums.
// Keep labels separate from wire values, and leave optional empty fields absent.
type elicitationField struct {
	Schema  map[string]any
	Values  []string
	Checked []bool
	Bool    bool
}

func elicitationQuestion(name string, schema map[string]any, required bool) question {
	q := question{ID: name, Header: fallback(str(schema, "title"), name), Text: str(schema, "description"), Type: str(schema, "type"), Required: required, Selected: -1, Editor: textEditor("", false), Form: &elicitationField{Schema: schema}}
	f := q.Form
	choices := schema
	if q.Type == "array" {
		choices, _ = schema["items"].(map[string]any)
	}
	if values, ok := choices["enum"].([]any); ok {
		names, _ := choices["enumNames"].([]any)
		for i, value := range values {
			s, ok := value.(string)
			if !ok {
				continue
			}
			label := s
			if i < len(names) {
				if n, ok := names[i].(string); ok {
					label = n
				}
			}
			f.Values = append(f.Values, s)
			q.Options = append(q.Options, label)
		}
	}
	for _, key := range []string{"oneOf", "anyOf"} {
		if values, ok := choices[key].([]any); ok {
			for _, value := range values {
				m, _ := value.(map[string]any)
				f.Values = append(f.Values, str(m, "const"))
				q.Options = append(q.Options, fallback(str(m, "title"), str(m, "const")))
			}
		}
	}
	if q.Type == "array" {
		f.Checked = make([]bool, len(f.Values))
		defaults, _ := schema["default"].([]any)
		for i, v := range f.Values {
			for _, d := range defaults {
				if d == v {
					f.Checked[i] = true
				}
			}
		}
	} else if len(f.Values) > 0 {
		for i, v := range f.Values {
			if schema["default"] == v {
				q.Selected = i
			}
		}
	} else if q.Type == "boolean" {
		f.Bool, _ = schema["default"].(bool)
	} else if d, ok := schema["default"]; ok {
		setText(q.Editor, fmt.Sprint(d))
	}
	q.Editor.Placeholder = map[string]string{"email": "name@example.com", "uri": "https://…", "date": "YYYY-MM-DD", "date-time": "YYYY-MM-DDThh:mm:ssZ"}[str(schema, "format")]
	return q
}

func (a *App) drawQuestion(w *nucular.Window, q *question) {
	header := q.Header
	if q.Required {
		header += " *"
	}
	muted(w, header+": "+q.Text, a.p)
	if q.Form != nil && q.Type == "boolean" {
		w.Row(28).Dynamic(1)
		w.CheckboxText(q.Header, &q.Form.Bool)
		return
	}
	for i, label := range q.Options {
		w.Row(28).Dynamic(1)
		if q.Form != nil && q.Type == "array" {
			w.CheckboxText(label, &q.Form.Checked[i])
		} else if button(w, label, q.Selected == i, a.p) {
			q.Selected = i
		}
		if i < len(q.Descriptions) && q.Descriptions[i] != "" {
			muted(w, q.Descriptions[i], a.p)
		}
	}
	if q.Form == nil || len(q.Options) == 0 {
		w.Row(28).Dynamic(1)
		q.Editor.Edit(w)
	}
}

func questionAnswers(q question) []string {
	answers := []string{}
	if q.Selected >= 0 && q.Selected < len(q.Options) {
		answers = append(answers, q.Options[q.Selected])
	}
	if note := strings.TrimSpace(text(q.Editor)); note != "" {
		answers = append(answers, "user_note: "+note)
	}
	return answers
}

func elicitationValue(q question) (any, error) {
	f := q.Form
	if q.Type == "boolean" {
		return f.Bool, nil
	}
	if q.Type == "array" {
		values := []string{}
		for i, value := range f.Values {
			if f.Checked[i] {
				values = append(values, value)
			}
		}
		minimum, _ := f.Schema["minItems"].(float64)
		if q.Required && minimum < 1 {
			minimum = 1
		}
		if float64(len(values)) < minimum {
			return nil, fmt.Errorf("choose at least %g options", minimum)
		}
		if max, ok := f.Schema["maxItems"].(float64); ok && float64(len(values)) > max {
			return nil, fmt.Errorf("choose at most %g options", max)
		}
		if len(values) == 0 {
			return nil, nil
		}
		return values, nil
	}
	if len(f.Values) > 0 {
		if q.Selected >= 0 && q.Selected < len(f.Values) {
			return f.Values[q.Selected], nil
		}
		if q.Required {
			return nil, fmt.Errorf("choose an option")
		}
		return nil, nil
	}
	value := strings.TrimSpace(text(q.Editor))
	if value == "" {
		if q.Required {
			return nil, fmt.Errorf("this field is required")
		}
		return nil, nil
	}
	if q.Type == "integer" || q.Type == "number" {
		n, err := strconv.ParseFloat(value, 64)
		if err != nil || math.IsInf(n, 0) || math.IsNaN(n) {
			return nil, fmt.Errorf("enter a finite number")
		}
		if minimum, ok := f.Schema["minimum"].(float64); ok && n < minimum {
			return nil, fmt.Errorf("enter a value of at least %g", minimum)
		}
		if maximum, ok := f.Schema["maximum"].(float64); ok && n > maximum {
			return nil, fmt.Errorf("enter a value of at most %g", maximum)
		}
		if q.Type == "integer" {
			i, err := strconv.ParseInt(value, 10, 64)
			if err != nil {
				return nil, fmt.Errorf("enter a whole number")
			}
			return i, nil
		}
		return n, nil
	}
	if q.Type != "string" {
		return nil, fmt.Errorf("unsupported field type %q", q.Type)
	}
	length := float64(utf8.RuneCountInString(value))
	if n, ok := f.Schema["minLength"].(float64); ok && length < n {
		return nil, fmt.Errorf("enter at least %g characters", n)
	}
	if n, ok := f.Schema["maxLength"].(float64); ok && length > n {
		return nil, fmt.Errorf("enter at most %g characters", n)
	}
	valid := true
	switch str(f.Schema, "format") {
	case "email":
		local, domain, ok := strings.Cut(value, "@")
		valid = ok && local != "" && strings.Contains(domain, ".") && !strings.Contains(domain, "@") && !strings.HasPrefix(domain, ".") && !strings.HasSuffix(domain, ".")
	case "uri":
		u, err := url.Parse(value)
		valid = err == nil && u.Scheme != "" && len(value) > len(u.Scheme)+1
	case "date":
		_, err := time.Parse("2006-01-02", value)
		valid = err == nil
	case "date-time":
		_, err := time.Parse(time.RFC3339, value)
		valid = err == nil
	}
	if format := str(f.Schema, "format"); format != "" {
		valid = valid && strings.IndexFunc(value, unicode.IsSpace) == -1
		if !valid {
			return nil, fmt.Errorf("enter a valid %s", format)
		}
	}
	return value, nil
}
