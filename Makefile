GO := env GOTOOLCHAIN=go1.26.0 go
.PHONY: build test check windows mac clean
build:
	mkdir -p build
	$(GO) build -trimpath -o build/fastrock ./cmd/fastrock
test:
	$(GO) test -race ./...
	cd third_party/nucular && $(GO) test -race ./...
check:
	$(GO) vet ./...
	$(GO) test -race ./...
	cd third_party/nucular && $(GO) test -race ./...
windows:
	mkdir -p build
	GOOS=windows GOARCH=amd64 CGO_ENABLED=0 $(GO) build -trimpath -ldflags='-H=windowsgui -s -w' -o build/fastrock.exe ./cmd/fastrock
mac:
	mkdir -p build/Fastrock.app/Contents/MacOS
	$(GO) build -trimpath -ldflags='-s -w' -o build/Fastrock.app/Contents/MacOS/fastrock ./cmd/fastrock
	cp packaging/macos/Info.plist build/Fastrock.app/Contents/Info.plist
clean:
	rm -rf build
