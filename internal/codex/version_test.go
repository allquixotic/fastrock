package codex

import "testing"

func TestV50MinimumCodexVersion(t *testing.T) {
	for _, tc := range []struct {
		version, minimum string
		valid            bool
	}{
		{"codex-cli 0.162.0", SupportedVersion, true},
		{"codex-cli 0.163.2-alpha.4", "0.163.2", true},
		{"codex-cli 0.163.1", "0.163.2", false},
		{"codex-cli 0.162.99", "0.163.0", false},
		{"codex-cli 1.0.0", "0.200.100", true},
		{"codex-cli 1.1.100", "1.2.0", false},
		{"codex-cli 1.2.3", "1.2.3", true},
		{"codex-cli 1.2.4", "1.2.3", true},
		{"codex-cli 2.0.0", "1.2.3", true},
		{"codex-cli 0.999.999", "1.0.0", false},
		{"codex-cli 9999999999999999999999999.0.0", "0.162.0", false},
		{"codex-cli 1.2.3", "invalid", false},
		{"codex-cli 1.2.3", "1.x.3", false},
		{"codex-cli 1.2.3", "1.2.9999999999999999999999", false},
		{"not a version", "0.162.0", false},
	} {
		if err := checkMinimumVersion(tc.version, tc.minimum); (err == nil) != tc.valid {
			t.Errorf("%q minimum %q: %v", tc.version, tc.minimum, err)
		}
	}
}
