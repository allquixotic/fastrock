package ui

import (
	"context"
	"path/filepath"
	"slices"
	"strings"

	"github.com/allquixotic/fastrock/internal/workspace"
)

// Both rows and tree nodes are immutable after publication. A metadata update
// copies only an AVL path, so a worker can capture the current root in O(1)
// without reading mutable conversations or copying the complete history.
type sidebarRow struct {
	ID, Title, Cwd, search string
	Updated                int64
	Archived               bool
}

func sidebarMetadata(c *workspace.Conversation) *sidebarRow {
	if c.SidebarHidden {
		return nil
	}
	return &sidebarRow{ID: c.ID, Title: c.Title, Cwd: c.Cwd, search: strings.ToLower(c.Title + " " + c.Cwd), Updated: c.Updated, Archived: c.Archived}
}

type sidebarNode struct {
	row         *sidebarRow
	left, right *sidebarNode
	height      int
}

func sidebarHeight(n *sidebarNode) int {
	if n == nil {
		return 0
	}
	return n.height
}

func sidebarBranch(row *sidebarRow, left, right *sidebarNode) *sidebarNode {
	return &sidebarNode{row: row, left: left, right: right, height: 1 + max(sidebarHeight(left), sidebarHeight(right))}
}

func sidebarBalance(row *sidebarRow, left, right *sidebarNode) *sidebarNode {
	if sidebarHeight(left)-sidebarHeight(right) > 1 {
		if sidebarHeight(left.left) < sidebarHeight(left.right) {
			pivot := left.right
			left = sidebarBranch(pivot.row, sidebarBranch(left.row, left.left, pivot.left), pivot.right)
		}
		return sidebarBranch(left.row, left.left, sidebarBranch(row, left.right, right))
	}
	if sidebarHeight(right)-sidebarHeight(left) > 1 {
		if sidebarHeight(right.right) < sidebarHeight(right.left) {
			pivot := right.left
			right = sidebarBranch(pivot.row, pivot.left, sidebarBranch(right.row, pivot.right, right.right))
		}
		return sidebarBranch(right.row, sidebarBranch(row, left, right.left), right.right)
	}
	return sidebarBranch(row, left, right)
}

func sidebarSet(n *sidebarNode, id string, row *sidebarRow) *sidebarNode {
	if n == nil {
		if row == nil {
			return nil
		}
		return sidebarBranch(row, nil, nil)
	}
	if id < n.row.ID {
		left := sidebarSet(n.left, id, row)
		if left == n.left {
			return n
		}
		return sidebarBalance(n.row, left, n.right)
	}
	if id > n.row.ID {
		right := sidebarSet(n.right, id, row)
		if right == n.right {
			return n
		}
		return sidebarBalance(n.row, n.left, right)
	}
	if row != nil {
		if *row == *n.row {
			return n
		}
		return sidebarBranch(row, n.left, n.right)
	}
	if n.left == nil {
		return n.right
	}
	if n.right == nil {
		return n.left
	}
	first := n.right
	for first.left != nil {
		first = first.left
	}
	return sidebarBalance(first.row, n.left, sidebarSet(n.right, first.row.ID, nil))
}

func visitSidebar(ctx context.Context, n *sidebarNode, visit func(*sidebarRow)) bool {
	if n == nil {
		return true
	}
	if ctx.Err() != nil {
		return false
	}
	if !visitSidebar(ctx, n.left, visit) {
		return false
	}
	visit(n.row)
	return visitSidebar(ctx, n.right, visit)
}

type sidebarPrepared struct {
	folders []sidebarFolder
	recent  []*sidebarRow
	count   int
}

func compareSidebar(a, b *sidebarRow) int {
	if a.Updated > b.Updated {
		return -1
	}
	if a.Updated < b.Updated {
		return 1
	}
	return strings.Compare(a.ID, b.ID)
}

func querySidebar(ctx context.Context, root *sidebarNode, query string, archived bool, matches map[string]bool) ([]*sidebarRow, []*sidebarRow, error) {
	query = strings.ToLower(query)
	var rows []*sidebarRow
	out := sidebarPrepared{}
	visitSidebar(ctx, root, func(row *sidebarRow) {
		if row.Archived == archived && (strings.Contains(row.search, query) || matches[row.ID]) {
			rows = append(rows, row)
		}
		if !row.Archived {
			at, _ := slices.BinarySearchFunc(out.recent, row, compareSidebar)
			if at < 8 {
				out.recent = slices.Insert(out.recent, at, row)
				out.recent = out.recent[:min(8, len(out.recent))]
			}
		}
	})
	if err := ctx.Err(); err != nil {
		return nil, nil, err
	}
	slices.SortFunc(rows, compareSidebar)
	if err := ctx.Err(); err != nil {
		return nil, nil, err
	}
	return rows, out.recent, nil
}

func prepareSidebar(ctx context.Context, root *sidebarNode, query string, archived bool, matches map[string]bool) (sidebarPrepared, error) {
	rows, recent, err := querySidebar(ctx, root, query, archived, matches)
	if err != nil {
		return sidebarPrepared{}, err
	}
	out := sidebarPrepared{recent: recent}
	byPath := map[string]int{}
	for _, row := range rows {
		i, exists := byPath[row.Cwd]
		if !exists {
			i = len(out.folders)
			out.folders = append(out.folders, sidebarFolder{path: row.Cwd, title: filepath.Base(row.Cwd)})
			byPath[row.Cwd] = i
		}
		out.folders[i].rows = append(out.folders[i].rows, row)
	}
	out.count = len(rows)
	return out, ctx.Err()
}
