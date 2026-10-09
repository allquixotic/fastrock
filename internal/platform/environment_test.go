package platform

import (
	"strings"
	"testing"
)

func TestChildrenDoNotInheritGUISecrets(t *testing.T) {
	for _, key := range []string{"FASTROCK_RALLY_TOKEN", "FASTROCK_BROKER_TOKEN", "FASTROCK_BROKER", "FASTROCK_AUTOMATION"} {
		t.Setenv(key, "secret")
	}
	for _, entry := range ChildEnv() {
		if strings.HasPrefix(strings.ToUpper(entry), "FASTROCK_RALLY_TOKEN=") || strings.HasPrefix(strings.ToUpper(entry), "FASTROCK_BROKER") || strings.HasPrefix(strings.ToUpper(entry), "FASTROCK_AUTOMATION=") {
			t.Fatal("GUI credential inherited")
		}
	}
}
func TestInstanceLockIsReleasedAndRejectsSecondWriter(t *testing.T) {
	dir := t.TempDir()
	release, err := AcquireInstance(dir)
	if err != nil {
		t.Fatal(err)
	}
	if second, err := AcquireInstance(dir); err == nil {
		second()
		release()
		t.Fatal("allowed two session writers")
	}
	release()
	next, err := AcquireInstance(dir)
	if err != nil {
		t.Fatal(err)
	}
	next()
}
