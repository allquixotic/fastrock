// Package buildinfo identifies a release. Unstamped local builds stay portable
// and never replace an installed copy automatically.
package buildinfo

var Version = "dev"

// ReleaseStamp is injected by the release linker and kept in a signed native
// read-only data section. It remains discoverable even with -trimpath -s -w.
var ReleaseStamp = "FastrockRelease[dev]"

func init() {
	const prefix = "FastrockRelease["
	if ReleaseStamp != "FastrockRelease[dev]" && len(ReleaseStamp) > len(prefix)+1 && ReleaseStamp[:len(prefix)] == prefix && ReleaseStamp[len(ReleaseStamp)-1] == ']' {
		Version = ReleaseStamp[len(prefix) : len(ReleaseStamp)-1]
	}
}
