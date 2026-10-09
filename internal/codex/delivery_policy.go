package codex

import (
	"encoding/json"
	"fmt"
	"path/filepath"
	"strings"
)

// PermissionScope contains effective settings, never requested model values or
// credentials. Roots is a JSON string to keep published thread rows comparable.
type PermissionScope struct {
	Approval, Sandbox, Roots string
	Network                  bool
}

func (p PermissionScope) ranks() (approval, sandbox int, known bool) {
	switch p.Approval {
	case "untrusted", "unless-trusted", "unlessTrusted":
		approval = 0
	case "on-request", "onRequest", "granular":
		approval = 1
	case "never":
		approval = 2
	default:
		return 0, 0, false
	}
	switch p.Sandbox {
	case "read-only", "readOnly":
		sandbox = 0
	case "workspace-write", "workspaceWrite":
		sandbox = 1
	case "danger-full-access", "dangerFullAccess", "external-sandbox", "externalSandbox":
		sandbox = 2
	default:
		return 0, 0, false
	}
	return approval, sandbox, true
}

func (p PermissionScope) network() bool {
	return p.Network || p.Sandbox == "danger-full-access" || p.Sandbox == "dangerFullAccess"
}

// DeliveryEscalation explains why a pair must ask every time. Missing or
// malformed effective settings never authorize an unattended delivery.
func DeliveryEscalation(from, to OpenThread) string {
	fa, fs, fk := from.Permissions.ranks()
	ta, ts, tk := to.Permissions.ranks()
	if !fk || !tk {
		return "the permissions of one of the tabs are not known yet"
	}
	var reasons []string
	if ts > fs {
		reasons = append(reasons, "the target has broader filesystem access than the sending tab")
	} else if ts == 1 && fs == 1 {
		var sourceRoots, targetRoots []string
		if json.Unmarshal([]byte(from.Permissions.Roots), &sourceRoots) != nil || json.Unmarshal([]byte(to.Permissions.Roots), &targetRoots) != nil || from.Cwd == "" || to.Cwd == "" {
			return "the writable folders of one of the tabs are not known yet"
		}
		sourceRoots = append(sourceRoots, from.Cwd)
		targetRoots = append(targetRoots, to.Cwd)
		for _, root := range targetRoots {
			allowed := false
			for _, base := range sourceRoots {
				if filepath.IsAbs(base) && filepath.IsAbs(root) {
					rel, err := filepath.Rel(base, root)
					allowed = allowed || err == nil && rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator))
				}
			}
			if !allowed {
				reasons = append(reasons, fmt.Sprintf("the target can write in %s, which the sending tab cannot", root))
				break
			}
		}
	}
	if to.Permissions.network() && !from.Permissions.network() {
		reasons = append(reasons, "the target has network access and the sending tab does not")
	}
	if ta > fa {
		reasons = append(reasons, "the target asks for approval less often than the sending tab")
	}
	return strings.Join(reasons, "; ")
}
