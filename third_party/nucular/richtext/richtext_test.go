package richtext

import (
	"testing"
)

func TestGetUsesRequestedRange(t *testing.T) {
	r := &RichText{chunks: []chunk{{s: "hello "}, {s: "world"}}, Sel: Sel{0, 1}}
	if got := r.Get(Sel{6, 11}); got != "world" {
		t.Fatalf("got %q", got)
	}
	if got := r.Get(Sel{0, 11}); got != "hello world" {
		t.Fatalf("got %q", got)
	}
}

func TestSeparateStyles(t *testing.T) {
	out := separateStyles([]styleSel{
		{Sel: Sel{0, 20}, align: 'a'},
		{Sel: Sel{10, 15}, align: 'b'},
		{Sel: Sel{18, 30}, align: 'c'},
		{Sel: Sel{25, 35}, align: 'd'},
		{Sel: Sel{40, 45}, align: 'e'},
		{Sel: Sel{50, 60}, align: 'f'},
		{Sel: Sel{55, 57}, align: 'g'},
		{Sel: Sel{100, 150}, align: 'h'},
		{Sel: Sel{110, 140}, align: 'i'},
		{Sel: Sel{120, 130}, align: 'j'},
	})

	tgt := []styleSel{
		{Sel: Sel{0, 10}, align: 'a'},
		{Sel: Sel{10, 15}, align: 'b'},
		{Sel: Sel{15, 18}, align: 'a'},
		{Sel: Sel{18, 25}, align: 'c'},
		{Sel: Sel{25, 35}, align: 'd'},
		{Sel: Sel{40, 45}, align: 'e'},
		{Sel: Sel{50, 55}, align: 'f'},
		{Sel: Sel{55, 57}, align: 'g'},
		{Sel: Sel{57, 60}, align: 'f'},
		{Sel: Sel{100, 110}, align: 'h'},
		{Sel: Sel{110, 120}, align: 'i'},
		{Sel: Sel{120, 130}, align: 'j'},
		{Sel: Sel{130, 140}, align: 'i'},
		{Sel: Sel{140, 150}, align: 'h'},
	}

	for i := range out {
		t.Logf("%d %#v %c\n", i, out[i].Sel, out[i].align)
	}

	if len(out) != len(tgt) {
		t.Fatalf("length mismatch\n")
	}

	for i := range out {
		if out[i].Sel != tgt[i].Sel || out[i].align != tgt[i].align {
			t.Fatalf("content mismatch at index %d\n", i)
		}
	}
}

func TestMergeStyles(t *testing.T) {
	link := func() {}
	out := mergeStyles(
		[]styleSel{ // styleSels
			{Sel: Sel{10, 20}, TextStyle: TextStyle{Flags: 'a'}},
			{Sel: Sel{50, 60}, TextStyle: TextStyle{Flags: 'b'}},
			{Sel: Sel{150, 170}, TextStyle: TextStyle{Flags: 'c'}},
		},
		[]styleSel{ // alignSels
			{Sel: Sel{0, 100}, align: AlignLeft},
		},
		[]styleSel{ // linkSels
			{Sel: Sel{10, 20}, link: link},
			{Sel: Sel{200, 210}, link: link},
		})

	tgt := []styleSel{
		{Sel: Sel{0, 10}, align: AlignLeft},
		{Sel: Sel{10, 20}, TextStyle: TextStyle{Flags: 'a'}, align: AlignLeft, link: link},
		{Sel: Sel{20, 50}, align: AlignLeft},
		{Sel: Sel{50, 60}, TextStyle: TextStyle{Flags: 'b'}, align: AlignLeft},
		{Sel: Sel{60, 100}, align: AlignLeft},
		{Sel: Sel{150, 170}, TextStyle: TextStyle{Flags: 'c'}},
		{Sel: Sel{200, 210}, link: link},
	}

	for i := range out {
		t.Logf("%d %#v flags:%d align:%d link:%p", i, out[i].Sel, out[i].Flags, out[i].align, out[i].link)
	}

	if len(out) != len(tgt) {
		t.Fatal("length mismatch")
	}

	for i := range out {
		if out[i].Sel != tgt[i].Sel || out[i].Flags != tgt[i].Flags || out[i].align != tgt[i].align || (out[i].link != nil) != (tgt[i].link != nil) {
			t.Fatalf("content mismatch at %d", i)
		}
	}
}
