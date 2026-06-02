// ContractGate v1 ingest — Go example (net/http, no third-party deps).
//
// Run against the public demo:
//
//	export CONTRACTGATE_API_KEY=cg_live_<your_key>
//	export CONTRACTGATE_CONTRACT_ID=<uuid>
//	go run go_example.go
package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
	"time"
)

func main() {
	baseURL    := getenv("CONTRACTGATE_BASE_URL", "https://contractgate.io")
	apiKey     := mustenv("CONTRACTGATE_API_KEY")
	contractID := mustenv("CONTRACTGATE_CONTRACT_ID")

	client := &http.Client{Timeout: 30 * time.Second}

	// -----------------------------------------------------------------------
	// JSON array body
	// -----------------------------------------------------------------------
	events := []map[string]any{
		{"user_id": "u_001", "event_type": "login",    "timestamp": 1_714_000_000},
		{"user_id": "u_002", "event_type": "purchase", "timestamp": 1_714_000_001, "amount": 49.99},
		{"user_id": "u_003", "event_type": "bad_type", "timestamp": 1_714_000_002}, // will fail
	}

	body, _ := json.Marshal(events)
	req, _ := http.NewRequest(http.MethodPost,
		fmt.Sprintf("%s/v1/ingest/%s", baseURL, contractID),
		bytes.NewReader(body),
	)
	req.Header.Set("X-Api-Key",       apiKey)
	req.Header.Set("Content-Type",    "application/json")
	req.Header.Set("Idempotency-Key", fmt.Sprintf("go-example-%d", time.Now().UnixMilli()))

	resp, err := client.Do(req)
	must(err)
	defer resp.Body.Close()

	respBody, _ := io.ReadAll(resp.Body)

	var result map[string]any
	json.Unmarshal(respBody, &result)
	fmt.Printf("JSON batch: status=%d  total=%.0f  passed=%.0f  failed=%.0f\n",
		resp.StatusCode, result["total"], result["passed"], result["failed"])
	fmt.Printf("  X-RateLimit-Remaining: %s\n", resp.Header.Get("X-RateLimit-Remaining"))

	if results, ok := result["results"].([]any); ok {
		for _, r := range results {
			ev := r.(map[string]any)
			mark := "✓"
			if ev["passed"] != true {
				mark = "✗"
			}
			qid := ""
			if ev["quarantine_id"] != nil {
				qid = fmt.Sprintf("  quarantine_id=%v", ev["quarantine_id"])
			}
			fmt.Printf("  [%s] index=%.0f%s\n", mark, ev["index"], qid)
		}
	}

	// -----------------------------------------------------------------------
	// NDJSON body
	// -----------------------------------------------------------------------
	ndjsonLines := []map[string]any{
		{"user_id": "u_nd1", "event_type": "view",  "timestamp": 1_714_000_200},
		{"user_id": "u_nd2", "event_type": "click", "timestamp": 1_714_000_201},
	}
	var sb strings.Builder
	for _, line := range ndjsonLines {
		b, _ := json.Marshal(line)
		sb.Write(b)
		sb.WriteByte('\n')
	}

	req2, _ := http.NewRequest(http.MethodPost,
		fmt.Sprintf("%s/v1/ingest/%s", baseURL, contractID),
		strings.NewReader(sb.String()),
	)
	req2.Header.Set("X-Api-Key",    apiKey)
	req2.Header.Set("Content-Type", "application/x-ndjson")

	resp2, err := client.Do(req2)
	must(err)
	defer resp2.Body.Close()

	body2, _ := io.ReadAll(resp2.Body)
	var r2 map[string]any
	json.Unmarshal(body2, &r2)
	fmt.Printf("\nNDJSON batch: status=%d  total=%.0f  passed=%.0f\n",
		resp2.StatusCode, r2["total"], r2["passed"])
}

func getenv(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

func mustenv(key string) string {
	v := os.Getenv(key)
	if v == "" {
		fmt.Fprintf(os.Stderr, "error: %s env var required\n", key)
		os.Exit(1)
	}
	return v
}

func must(err error) {
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}
