package ui

import (
	"context"
	"fmt"
	"sync"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"golang.org/x/sync/singleflight"
)

type cachedSkills struct {
	list    skillList
	expires time.Time
}
type skillCache struct {
	mu         sync.Mutex
	values     map[string]cachedSkills
	generation uint64
	flights    singleflight.Group
}

func (c *skillCache) invalidate() {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.generation++
	c.values = nil
}
func (c *skillCache) get(ctx, lifetime context.Context, client *codex.Client, cwd string) (skillList, error) {
	c.mu.Lock()
	key := fmt.Sprintf("%p:%d:%s", client, c.generation, cwd)
	generation := c.generation
	entry, ok := c.values[key]
	c.mu.Unlock()
	if ok && time.Now().Before(entry.expires) {
		return entry.list, nil
	}
	result := c.flights.DoChan(key, func() (any, error) {
		work, cancel := context.WithTimeout(lifetime, 10*time.Second)
		defer cancel()
		var list skillList
		if err := client.Call(work, "skills/list", map[string]any{"cwds": []string{cwd}}, &list); err != nil {
			return skillList{}, err
		}
		c.mu.Lock()
		defer c.mu.Unlock()
		if generation == c.generation {
			if len(c.values) >= 32 || c.values == nil {
				c.values = map[string]cachedSkills{}
			}
			c.values[key] = cachedSkills{list, time.Now().Add(time.Minute)}
		}
		return list, nil
	})
	select {
	case <-ctx.Done():
		return skillList{}, ctx.Err()
	case r := <-result:
		if r.Err != nil {
			return skillList{}, r.Err
		}
		return r.Val.(skillList), nil
	}
}
