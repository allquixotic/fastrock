package desktop

import (
	"image"

	"github.com/allquixotic/fastrock/internal/desktop/command"
)

// commandDamage includes both the old and new footprint. In particular, an
// unchanged command under a changed scissor must be repainted too. It is safe
// to overestimate damage; underestimating leaves stale pixels on screen.
func commandDamage(previous, current []command.Command, bounds image.Rectangle) (damage image.Rectangle, effects bool) {
	oldClip, newClip := bounds, bounds
	if len(previous) == 0 {
		damage = bounds
	}
	for i := 0; i < max(len(previous), len(current)); i++ {
		var old, next *command.Command
		if i < len(previous) {
			old = &previous[i]
			if old.Kind == command.ScissorCmd {
				oldClip = bounds.Intersect(old.Rectangle())
			}
		}
		if i < len(current) {
			next = &current[i]
			if next.Kind == command.ScissorCmd {
				newClip = bounds.Intersect(next.Rectangle())
			}
			switch next.Kind {
			case command.SetClipboardCmd, command.GetClipboardCmd, command.GetPrimarySelectionCmd:
				effects = true
			}
		}
		if old != nil && next != nil && *old == *next && oldClip == newClip {
			continue
		}
		if old != nil {
			damage = damage.Union(commandBounds(old).Intersect(oldClip))
		}
		if next != nil {
			damage = damage.Union(commandBounds(next).Intersect(newClip))
		}
	}
	return damage.Intersect(bounds), effects
}

func commandBounds(c *command.Command) image.Rectangle {
	switch c.Kind {
	case command.RectFilledCmd, command.CircleFilledCmd, command.ImageCmd, command.TextCmd:
		return c.Rectangle()
	case command.LineCmd:
		p, q := c.Line.Begin, c.Line.End
		pad := int(c.Line.LineThickness)/2 + 2
		return image.Rect(min(p.X, q.X)-pad, min(p.Y, q.Y)-pad, max(p.X, q.X)+pad, max(p.Y, q.Y)+pad)
	case command.TriangleFilledCmd:
		p, q, r := c.TriangleFilled.A, c.TriangleFilled.B, c.TriangleFilled.C
		return image.Rect(min(p.X, min(q.X, r.X))-1, min(p.Y, min(q.Y, r.Y))-1, max(p.X, max(q.X, r.X))+2, max(p.Y, max(q.Y, r.Y))+2)
	default:
		return image.Rectangle{}
	}
}
