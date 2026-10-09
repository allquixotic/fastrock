package main

import (
	"flag"
	"fmt"
	"github.com/allquixotic/fastrock/internal/mockrally"
	"net"
	"net/http"
	"os"
)

func main() {
	address := flag.String("listen", "127.0.0.1:18081", "Loopback listen address")
	flag.Parse()
	host, _, e := net.SplitHostPort(*address)
	if e != nil || net.ParseIP(host) == nil || !net.ParseIP(host).IsLoopback() {
		fmt.Fprintln(os.Stderr, "mock server must bind a loopback IP")
		os.Exit(1)
	}
	fmt.Printf("Mock Rally: http://%s; token: mock-token (fictional data only)\n", *address)
	if e = http.ListenAndServe(*address, mockrally.New()); e != nil {
		fmt.Fprintln(os.Stderr, e)
		os.Exit(1)
	}
}
