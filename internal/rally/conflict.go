package rally

import "strings"

// ConflictError distinguishes a stale reviewed revision from transport errors.
type ConflictError struct{ Ref string }

func (e *ConflictError) Error() string {
	return "item changed on Rally; local edits are retained, reload before retrying"
}

// SameReference compares concrete object identities after applying the client's
// origin/path checks, allowing WSAPI's absolute and relative representations.
func (c *Client) SameReference(a, b string) bool {
	leftKind, leftOK := c.ReferenceKind(a)
	rightKind, rightOK := c.ReferenceKind(b)
	if !leftOK || !rightOK || leftKind != rightKind {
		return false
	}
	left, _ := c.resolve(a)
	right, _ := c.resolve(b)
	return strings.EqualFold(left.Path, right.Path)
}
