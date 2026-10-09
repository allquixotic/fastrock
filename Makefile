GO := env CGO_ENABLED=1 GOTOOLCHAIN=go1.27.2 go
WINDOWS_CC ?= x86_64-w64-mingw32-gcc
WINDOWS_CXX ?= x86_64-w64-mingw32-g++
# Local builds remain outside the release update channel, while identifying the
# exact checkout. Release packaging supplies its own signed release stamp.
VERSION ?= dev-$(shell git describe --always --dirty 2>/dev/null || printf unknown)
LDFLAGS := -X github.com/allquixotic/fastrock/internal/buildinfo.Version=$(VERSION)
.PHONY: build test check windows mac clean
build:
	mkdir -p build
	$(GO) build -trimpath -ldflags='$(LDFLAGS)' -o build/fastrock ./cmd/fastrock
	$(GO) run dev/check-linkage.go build/fastrock
test:
	$(GO) test -tags=fltk_headless ./...
check:
	$(GO) vet -tags=fltk_headless ./...
	$(GO) test -tags=fltk_headless ./...
windows:
	mkdir -p build
	GOOS=windows GOARCH=amd64 CC=$(WINDOWS_CC) CXX=$(WINDOWS_CXX) $(GO) build -trimpath -ldflags='$(LDFLAGS) -H=windowsgui -s -w -extldflags "-static -static-libgcc -static-libstdc++"' -o build/fastrock.exe ./cmd/fastrock
	$(GO) run dev/check-linkage.go build/fastrock.exe
mac:
	mkdir -p build/Fastrock.app/Contents/MacOS
	$(GO) build -trimpath -ldflags='$(LDFLAGS) -s -w' -o build/Fastrock.app/Contents/MacOS/fastrock ./cmd/fastrock
	$(GO) run dev/check-linkage.go build/Fastrock.app/Contents/MacOS/fastrock
	sed 's/@FASTROCK_VERSION@/0.0.0/g' packaging/macos/Info.plist > build/Fastrock.app/Contents/Info.plist
clean:
	rm -rf build
