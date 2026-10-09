package ui

import (
	"fmt"
	"sort"
	"strings"
)

func object(v any) map[string]any { m, _ := v.(map[string]any); return m }
func objects(v any) []map[string]any {
	values, _ := v.([]any)
	result := make([]map[string]any, 0, len(values))
	for _, value := range values {
		if m := object(value); m != nil {
			result = append(result, m)
		}
	}
	return result
}
func flag(m map[string]any, key string) bool { b, _ := m[key].(bool); return b }

// These projections retain server order within groups where order has meaning
// (notably hook displayOrder), and never turn diagnostic objects into actions.
func extensionItems(page string, data any) ([]settingsItem, bool) {
	root := object(data)
	switch page {
	case "Features":
		return featureItems(root), true
	case "Skills":
		return skillItems(root), true
	case "Hooks":
		return hookItems(root), true
	case "Plugins":
		return pluginItems(root), true
	case "MCP servers":
		return mcpItems(root), true
	}
	return nil, false
}
func featureItems(root map[string]any) []settingsItem {
	stages := []struct{ key, title string }{{"beta", "Beta"}, {"stable", "Stable"}, {"underDevelopment", "Under development (may be unstable)"}, {"deprecated", "Deprecated"}}
	groups := map[string][]settingsItem{}
	for _, m := range objects(root["data"]) {
		stage := str(m, "stage")
		if stage == "removed" {
			continue
		}
		id := str(m, "name")
		if id == "" {
			continue
		}
		on, def := flag(m, "enabled"), flag(m, "defaultEnabled")
		item := settingsItem{ID: id, Name: fallback(str(m, "displayName"), id), Description: fallback(str(m, "description"), str(m, "announcement")), Enabled: on, Raw: m, Detail: fmt.Sprintf("features.%s · default %s", id, map[bool]string{true: "on", false: "off"}[def])}
		if on != def {
			item.Status = "Changed"
		}
		groups[stage] = append(groups[stage], item)
	}
	known := map[string]bool{}
	var result []settingsItem
	appendGroup := func(key, title string) {
		items := groups[key]
		sort.Slice(items, func(i, j int) bool { return items[i].ID < items[j].ID })
		for _, item := range items {
			item.Group = title
			result = append(result, item)
		}
		known[key] = true
	}
	for _, stage := range stages {
		appendGroup(stage.key, stage.title)
	}
	var unknown []string
	for key := range groups {
		if !known[key] {
			unknown = append(unknown, key)
		}
	}
	sort.Strings(unknown)
	for _, key := range unknown {
		appendGroup(key, "Other features")
	}
	return result
}
func problemItems(group string, values any, pathKey string) []settingsItem {
	var result []settingsItem
	for i, m := range objects(values) {
		path := str(m, pathKey)
		result = append(result, settingsItem{Group: group, ID: fmt.Sprintf("error:%s:%d", path, i), Name: fallback(path, "Load error"), Error: str(m, "message"), ReadOnly: true, Raw: m})
	}
	return result
}
func skillItems(root map[string]any) []settingsItem {
	groups := map[string][]settingsItem{}
	seen := map[string]bool{}
	var problems []settingsItem
	for _, entry := range objects(root["data"]) {
		for _, m := range objects(entry["skills"]) {
			id := str(m, "path")
			if id == "" || seen[id] {
				continue
			}
			seen[id] = true
			face := object(m["interface"])
			item := settingsItem{ID: id, Name: fallback(str(face, "displayName"), str(m, "name")), Subtitle: id, Description: fallback(str(m, "shortDescription"), fallback(str(face, "shortDescription"), str(m, "description"))), Enabled: flag(m, "enabled"), Raw: m}
			if item.Enabled {
				item.Status = "Enabled"
			} else {
				item.Status = "Disabled"
			}
			if plugin := str(m, "pluginId"); plugin != "" {
				item.Detail = "Plugin " + plugin
			}
			groups[str(m, "scope")] = append(groups[str(m, "scope")], item)
		}
		problems = append(problems, problemItems("Skills that could not be loaded", entry["errors"], "path")...)
	}
	var result []settingsItem
	order := []struct{ key, title string }{{"repo", "Project skills"}, {"user", "Your skills"}, {"admin", "Managed skills"}, {"system", "Built-in skills"}}
	appendGroup := func(key, title string) {
		rows := groups[key]
		sort.Slice(rows, func(i, j int) bool { return rows[i].Name < rows[j].Name })
		for _, row := range rows {
			row.Group = title
			result = append(result, row)
		}
		delete(groups, key)
	}
	for _, group := range order {
		appendGroup(group.key, group.title)
	}
	var unknown []string
	for key := range groups {
		unknown = append(unknown, key)
	}
	sort.Strings(unknown)
	for _, key := range unknown {
		appendGroup(key, "Other skills")
	}
	return append(result, problems...)
}
func hookItems(root map[string]any) []settingsItem {
	var hooks []map[string]any
	var problems []settingsItem
	for _, entry := range objects(root["data"]) {
		hooks = append(hooks, objects(entry["hooks"])...)
		problems = append(problems, problemItems("Problems", entry["errors"], "path")...)
		warnings, _ := entry["warnings"].([]any)
		for i, value := range warnings {
			if warning, ok := value.(string); ok {
				problems = append(problems, settingsItem{Group: "Problems", ID: fmt.Sprintf("warning:%d:%d", len(problems), i), Name: "Warning", Description: warning, Status: "Warning", ReadOnly: true})
			}
		}
	}
	sort.SliceStable(hooks, func(i, j int) bool {
		a, _ := hooks[i]["displayOrder"].(float64)
		b, _ := hooks[j]["displayOrder"].(float64)
		if a != b {
			return a < b
		}
		return str(hooks[i], "key") < str(hooks[j], "key")
	})
	var result []settingsItem
	seen := map[string]bool{}
	for _, m := range hooks {
		id := str(m, "key")
		if id == "" || seen[id] {
			continue
		}
		seen[id] = true
		item := settingsItem{ID: id, Name: str(m, "eventName"), Enabled: flag(m, "enabled"), Description: str(m, "statusMessage"), ReadOnly: flag(m, "isManaged"), Raw: m, Group: "Hooks"}
		if matcher := str(m, "matcher"); matcher != "" {
			item.Name += " · " + matcher
		}
		switch str(m, "handlerType") {
		case "command":
			item.Subtitle = str(m, "command")
			if flag(m, "async") {
				item.Subtitle += " (async)"
			}
		case "mcpTool":
			item.Subtitle = "MCP tool " + str(m, "server") + "/" + str(m, "tool")
		case "prompt":
			item.Subtitle = "Prompt hook"
		case "agent":
			item.Subtitle = "Agent hook"
		}
		trust := str(m, "trustStatus")
		switch trust {
		case "managed":
			item.Status = "Managed"
		case "trusted":
			item.Status = "Trusted"
		case "untrusted":
			item.Status = "Not trusted"
		case "modified":
			item.Status = "Changed since trusted"
		}
		item.NeedsTrust = !item.ReadOnly && (trust == "untrusted" || trust == "modified") && str(m, "currentHash") != ""
		source := str(m, "source")
		if plugin := str(m, "pluginId"); plugin != "" {
			source = "Plugin " + plugin
		}
		item.Detail = source + " · " + str(m, "sourcePath")
		if timeout, ok := m["timeoutSec"].(float64); ok {
			item.Detail += fmt.Sprintf(" · timeout %gs", timeout)
		}
		result = append(result, item)
	}
	return append(result, problems...)
}
func pluginItems(root map[string]any) []settingsItem {
	var result []settingsItem
	for _, market := range objects(root["marketplaces"]) {
		face := object(market["interface"])
		group := fallback(str(face, "displayName"), str(market, "name"))
		plugins := objects(market["plugins"])
		if len(plugins) == 0 {
			result = append(result, settingsItem{Group: group, Name: "No plugins", ReadOnly: true})
		}
		for _, original := range plugins {
			m := make(map[string]any, len(original)+2)
			for k, v := range original {
				m[k] = v
			}
			m["_marketplacePath"], m["_marketplaceName"] = market["path"], market["name"]
			face := object(m["interface"])
			available := str(m, "availability") == "AVAILABLE" || str(m, "availability") == "ENABLED" || str(m, "availability") == ""
			item := settingsItem{ID: str(m, "id"), Name: fallback(str(face, "displayName"), str(m, "name")), Description: str(face, "shortDescription"), Subtitle: str(m, "id"), Detail: str(face, "developerName"), Group: group, Enabled: flag(m, "enabled"), Raw: m, ReadOnly: !available}
			installed := flag(m, "installed")
			item.CanRemove = installed
			item.CanInstall = !installed && available && str(m, "installPolicy") != "NOT_AVAILABLE"
			switch {
			case !available:
				item.Status = "Disabled by admin"
			case !installed:
				item.Status = "Not installed"
			case item.Enabled:
				item.Status = "Enabled"
			default:
				item.Status = "Installed"
			}
			if version := fallback(str(m, "localVersion"), str(m, "version")); version != "" {
				item.Subtitle += " · v" + version
			}
			result = append(result, item)
		}
	}
	return append(result, problemItems("Marketplaces that could not be loaded", root["marketplaceLoadErrors"], "marketplacePath")...)
}

func mcpItems(root map[string]any) []settingsItem {
	configuration := object(root["_configuration"])
	config := object(configuration["config"])
	servers := object(config["mcp_servers"])
	statuses := map[string]map[string]any{}
	for _, m := range objects(root["data"]) {
		if name := str(m, "name"); name != "" {
			statuses[name] = m
		}
	}
	names := map[string]bool{}
	for name := range statuses {
		names[name] = true
	}
	for name := range servers {
		names[name] = true
	}
	var sorted []string
	for name := range names {
		sorted = append(sorted, name)
	}
	sort.Strings(sorted)
	user := map[string]bool{}
	for _, layer := range objects(configuration["layers"]) {
		if str(layer, "disabledReason") != "" || str(object(layer["name"]), "type") != "user" {
			continue
		}
		for name := range object(object(layer["config"])["mcp_servers"]) {
			user[name] = true
		}
	}
	var result []settingsItem
	for _, name := range sorted {
		status, conf := statuses[name], object(servers[name])
		enabled := true
		if b, ok := conf["enabled"].(bool); ok {
			enabled = b
		}
		raw := make(map[string]any, len(status)+2)
		for k, v := range status {
			raw[k] = v
		}
		raw["name"], raw["enabled"] = name, enabled
		item := settingsItem{ID: name, Name: name, Enabled: enabled, Raw: raw, ReadOnly: !user[name], CanRemove: user[name], Error: str(status, "toolsError")}
		item.Status = mcpStatus(status, enabled)
		if endpoint := str(conf, "url"); endpoint != "" {
			item.Subtitle = endpoint
		} else if cmd := str(conf, "command"); cmd != "" {
			words := []string{cmd}
			for _, arg := range func() []any { v, _ := conf["args"].([]any); return v }() {
				if word, ok := arg.(string); ok {
					words = append(words, word)
				}
			}
			item.Subtitle = displayCommand(words)
		} else {
			item.Subtitle = str(status, "httpOrigin")
		}
		origin, _ := configOrigin(configuration, "mcp_servers."+configKey(name))
		if plugin := str(status, "pluginId"); plugin != "" {
			item.Detail = "Plugin " + plugin
		} else if user[name] {
			item.Detail = "User config"
		} else if origin != "default" {
			item.Detail = origin
		}
		if tools, ok := status["tools"].(map[string]any); ok && item.Error == "" && (len(tools) > 0 || str(status, "runtimeStatus") == "connected") {
			item.Detail += fmt.Sprintf(" · %d tools", len(tools))
		}
		auth := str(status, "authStatus")
		if auth != "" && auth != "unknown" && auth != "unsupported" {
			item.Detail += " · auth: " + auth
		}
		item.Detail = strings.TrimPrefix(item.Detail, " · ")
		item.CanLogin = enabled && (auth == "notLoggedIn" || str(status, "runtimeStatus") == "authenticationRequired") && (str(conf, "url") != "" || str(status, "httpOrigin") != "")
		result = append(result, item)
	}
	return result
}
func mcpStatus(status map[string]any, enabled bool) string {
	if !enabled {
		return "Disabled"
	}
	labels := map[string]string{"connected": "Connected", "starting": "Starting", "authenticationRequired": "Sign-in required", "failed": "Failed", "cancelled": "Cancelled", "disabled": "Disabled", "notStarted": "Not started"}
	if label := labels[str(status, "runtimeStatus")]; label != "" {
		return label
	}
	if str(status, "toolsError") != "" {
		return "Failed"
	}
	if str(status, "authStatus") == "notLoggedIn" {
		return "Sign-in required"
	}
	if tools := object(status["tools"]); len(tools) > 0 {
		return "Available"
	}
	return "Not started"
}
