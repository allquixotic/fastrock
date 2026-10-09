package nucular

import (
	"bytes"
	"image"
	"image/color"
	"math/rand/v2"
	"testing"

	"github.com/aarzilli/nucular/command"
	"github.com/aarzilli/nucular/font"
	"github.com/aarzilli/nucular/internal/windowing"
	"github.com/aarzilli/nucular/rect"
)

func damageScene() []command.Command {
	b := &command.Buffer{}
	b.Reset()
	b.FillRect(rect.Rect{W: 240, H: 180}, 0, color.RGBA{20, 25, 30, 255})
	b.PushScissor(rect.Rect{X: 5, Y: 7, W: 180, H: 160})
	b.FillRect(rect.Rect{X: 20, Y: 22, W: 100, H: 90}, 12, color.RGBA{160, 30, 50, 180})
	b.FillCircle(rect.Rect{X: 50, Y: 50, W: 100, H: 70}, color.RGBA{40, 150, 70, 90})
	b.FillTriangle(image.Pt(25, 30), image.Pt(100, 145), image.Pt(170, 40), color.RGBA{180, 100, 50, 150})
	b.StrokeLine(image.Pt(0, 10), image.Pt(200, 140), 3, color.RGBA{255, 255, 255, 120})
	b.DrawText(rect.Rect{X: 30, Y: 40, W: 130, H: 40}, "Hi 世界!", font.DefaultFont(14, 1), color.RGBA{250, 250, 250, 255})
	return b.Commands
}

func TestV4DamageMatchesFullRedraw(t *testing.T) {
	original := damageScene()
	bounds := image.Rect(0, 0, 240, 180)
	for _, name := range []string{"color", "moved", "scissor", "removed", "inserted", "text", "line"} {
		t.Run(name, func(t *testing.T) {
			next := append([]command.Command(nil), original...)
			switch name {
			case "color":
				next[2].RectFilled.Color = color.RGBA{55, 200, 20, 140}
			case "moved":
				next[3].X += 31
				next[3].Y -= 20
			case "scissor":
				next[1].W -= 75
				next[1].Y += 40
			case "removed":
				next = append(next[:2], next[3:]...)
			case "inserted":
				next = append(next, original[2])
			case "text":
				next[6].Text.String = "changed!"
			case "line":
				next[5].Line.End = image.Pt(10, 0)
			}
			got, want := image.NewRGBA(bounds), image.NewRGBA(bounds)
			ctx := &context{cmds: original}
			ctx.Draw(got)
			damage, _ := commandDamage(original, next, bounds)
			if damage.Empty() || damage == bounds {
				t.Fatalf("not a bounded change: %v", damage)
			}
			ctx.cmds = next
			ctx.DrawDamage(got, damage)
			ctx.Draw(want)
			if !bytes.Equal(got.Pix, want.Pix) {
				for y := 0; y < bounds.Dy(); y++ {
					for x := 0; x < bounds.Dx(); x++ {
						if got.RGBAAt(x, y) != want.RGBAAt(x, y) {
							t.Fatalf("partial != full at %d,%d: %v != %v; damage %v", x, y, got.RGBAAt(x, y), want.RGBAAt(x, y), damage)
						}
					}
				}
			}
		})
	}
}

func TestV4DamageNoChangeAndClipboard(t *testing.T) {
	scene := damageScene()
	if r, effects := commandDamage(scene, scene, image.Rect(0, 0, 240, 180)); !r.Empty() || effects {
		t.Fatal(r, effects)
	}
	next := append(append([]command.Command{}, scene...), command.Command{Kind: command.GetClipboardCmd})
	if r, effects := commandDamage(scene, next, image.Rect(0, 0, 240, 180)); !r.Empty() || !effects {
		t.Fatal(r, effects)
	}
}

func TestV4DamageRandomClippedScenes(t *testing.T) {
	rng := rand.New(rand.NewPCG(1, 2))
	bounds := image.Rect(0, 0, 260, 180)
	for trial := 0; trial < 100; trial++ {
		b := &command.Buffer{}
		b.Reset()
		b.FillRect(rect.FromRectangle(bounds), 0, color.RGBA{10, 20, 30, 255})
		for n := 0; n < 30; n++ {
			if n%7 == 0 {
				b.PushScissor(rect.Rect{X: rng.IntN(30), Y: rng.IntN(20), W: 220, H: 160})
			}
			r := rect.Rect{X: rng.IntN(180), Y: rng.IntN(100), W: 30 + rng.IntN(50), H: 30 + rng.IntN(50)}
			c := color.RGBA{uint8(rng.IntN(256)), uint8(rng.IntN(256)), uint8(rng.IntN(256)), uint8(50 + rng.IntN(206))}
			if n%2 == 0 {
				b.FillCircle(r, c)
			} else {
				b.FillRect(r, 5, c)
				b.FillRect(rect.Rect{X: r.X + 2, Y: r.Y + 2, W: r.W - 4, H: r.H - 4}, 3, c)
			}
		}
		old := b.Commands
		next := append([]command.Command(nil), old...)
		i := 1 + rng.IntN(len(next)-1)
		if next[i].Kind == command.ScissorCmd {
			next[i].W -= 30
		} else {
			next[i].X += 10
			next[i].Y -= 8
		}
		got, want := image.NewRGBA(bounds), image.NewRGBA(bounds)
		ctx := &context{cmds: old}
		ctx.Draw(got)
		damage, _ := commandDamage(old, next, bounds)
		ctx.cmds = next
		ctx.DrawDamage(got, damage)
		ctx.Draw(want)
		if !bytes.Equal(got.Pix, want.Pix) {
			t.Fatalf("scene %d command %d damage %v differs", trial, i, damage)
		}
	}
}

func TestV7ShapeMaskCacheBounded(t *testing.T) {
	c := shapeMaskCache{}
	a := c.get(12, 12, 0)
	if a != c.get(12, 12, 0) {
		t.Fatal("cached mask replaced")
	}
	if a.AlphaAt(0, 0).A != 0 || a.AlphaAt(11, 11).A < 250 {
		t.Fatalf("corner mask orientation: outer=%d inner=%d", a.AlphaAt(0, 0).A, a.AlphaAt(11, 11).A)
	}
	for i := 10; i < 100; i++ {
		c.get(i, i, -1)
	}
	if len(c.entries) > 16 || c.bytes > 1<<20 {
		t.Fatal("mask cache unbounded", len(c.entries), c.bytes)
	}
	if c.get(513, 100, -1) != nil {
		t.Fatal("oversized mask cached")
	}
}

func TestV4NativeFractionalWheel(t *testing.T) {
	w := &masterWindow{}
	w.ctx = &context{}
	for i := 0; i < 8; i++ {
		w.handleEventLocked(windowing.Wheel{X: 10, Y: 20, DeltaX: -0.125, DeltaY: 0.125})
	}
	if m := w.ctx.Input.Mouse; m.ScrollDelta != 1 || m.ScrollDeltaX != -1 || m.Pos != image.Pt(10, 20) {
		t.Fatal(m)
	}
}

func BenchmarkV4DamageDraw(b *testing.B) {
	scene := damageScene()
	pixels := image.NewRGBA(image.Rect(0, 0, 240, 180))
	ctx := &context{cmds: scene}
	ctx.Draw(pixels)
	for _, damage := range []image.Rectangle{pixels.Bounds(), image.Rect(30, 40, 50, 60)} {
		name := "full"
		if damage != pixels.Bounds() {
			name = "partial"
		}
		b.Run(name, func(b *testing.B) {
			for i := 0; i < b.N; i++ {
				ctx.DrawDamage(pixels, damage)
			}
		})
	}
}
