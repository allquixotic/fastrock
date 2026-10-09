package main

import (
	"flag"
	"fmt"
	"github.com/allquixotic/fastrock/internal/mockmodel"
	"net"
	"net/http"
	"os"
)

func main() {
	address := flag.String("listen", "127.0.0.1:18080", "Loopback listen address")
	flag.Parse()
	host, _, e := net.SplitHostPort(*address)
	if e != nil || net.ParseIP(host) == nil || !net.ParseIP(host).IsLoopback() {
		fmt.Fprintln(os.Stderr, "mock server must bind a loopback IP")
		os.Exit(1)
	}
	fmt.Printf("Mock Responses API: http://%s/v1\n", *address)
	if e = http.ListenAndServe(*address, &mockmodel.Server{}); e != nil {
		fmt.Fprintln(os.Stderr, e)
		os.Exit(1)
	}
}
