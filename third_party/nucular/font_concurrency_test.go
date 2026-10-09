package nucular

import (
	"strings"
	"sync"
	"testing"

	"github.com/aarzilli/nucular/font"
)

func TestFontMeasurementsRemainConsistentAcrossWorkers(t *testing.T) {
	faces := []font.Face{font.DefaultFont(12, 1), font.DefaultFont(18, 1)}
	lines := []string{"Hello world", "日本語の文字", "A long line\nwith a second line", strings.Repeat("wide ", 1000)}
	want := make([][]int, len(faces))
	for i, f := range faces {
		want[i] = make([]int, len(lines))
		for j, s := range lines {
			want[i][j] = FontWidth(f, s)
		}
	}
	var wg sync.WaitGroup
	for range 8 {
		wg.Go(func() {
			for range 30 {
				for i, f := range faces {
					for j, s := range lines {
						if got := FontWidth(f, s); got != want[i][j] {
							t.Errorf("width changed: %d != %d", got, want[i][j])
						}
					}
				}
			}
		})
	}
	wg.Wait()
}
