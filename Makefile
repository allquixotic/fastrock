GO := env CGO_ENABLED=0 GOTOOLCHAIN=go1.27.2 go
.PHONY: build test check windows mac clean
build:
	mkdir -p build
	$(GO) build -trimpath -o build/fastrock ./cmd/fastrock
test:
	$(GO) test -tags=nucular_headless ./...
	cd third_party/nucular && $(GO) test -tags=nucular_headless ./...
check:
	$(GO) vet -tags=nucular_headless ./...
	$(GO) test -tags=nucular_headless ./...
	cd third_party/nucular && $(GO) test -tags=nucular_headless ./...
windows:
	mkdir -p build
	GOOS=windows GOARCH=amd64 CGO_ENABLED=0 $(GO) build -trimpath -ldflags='-H=windowsgui -s -w' -o build/fastrock.exe ./cmd/fastrock
mac:
	mkdir -p build/Fastrock.app/Contents/MacOS
	$(GO) build -trimpath -ldflags='-s -w' -o build/Fastrock.app/Contents/MacOS/fastrock ./cmd/fastrock
	cp packaging/macos/Info.plist build/Fastrock.app/Contents/Info.plist
clean:
	rm -rf build
