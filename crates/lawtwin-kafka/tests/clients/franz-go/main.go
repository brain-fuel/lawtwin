// Command franzgo-scenarios drives the LawTwin Kafka twin with franz-go.
// Each scenario checks what the client observes and prints a JSON summary;
// the Rust test that runs it then checks the contract's laws over the
// twin's history.
//
//	go run . <scenario> <bootstrap> <admin url>
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os"
	"sort"
	"sync"
	"time"

	"github.com/twmb/franz-go/pkg/kadm"
	"github.com/twmb/franz-go/pkg/kgo"
)

var (
	bootstrap string
	admin     string
)

// control calls the twin's admin API and gives the status and JSON body.
func control(method, path string, body interface{}) (int, map[string]interface{}) {
	var reader *bytes.Reader
	if body != nil {
		b, _ := json.Marshal(body)
		reader = bytes.NewReader(b)
	} else {
		reader = bytes.NewReader(nil)
	}
	req, _ := http.NewRequest(method, admin+path, reader)
	req.Header.Set("content-type", "application/json")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		fail("admin: %v", err)
	}
	defer resp.Body.Close()
	out := map[string]interface{}{}
	_ = json.NewDecoder(resp.Body).Decode(&out)
	return resp.StatusCode, out
}

func fail(format string, args ...interface{}) {
	fmt.Fprintf(os.Stderr, format+"\n", args...)
	os.Exit(1)
}

func check(ok bool, format string, args ...interface{}) {
	if !ok {
		fail(format, args...)
	}
}

func must(err error, what string) {
	if err != nil {
		fail("%s: %v", what, err)
	}
}

func until(what string, d time.Duration, test func() bool) {
	end := time.Now().Add(d)
	for !test() {
		if time.Now().After(end) {
			fail("timed out waiting for %s", what)
		}
		time.Sleep(50 * time.Millisecond)
	}
}

func client(opts ...kgo.Opt) *kgo.Client {
	base := []kgo.Opt{kgo.SeedBrokers(bootstrap), kgo.RetryTimeout(30 * time.Second), kgo.RecordPartitioner(kgo.ManualPartitioner())}
	cl, err := kgo.NewClient(append(base, opts...)...)
	must(err, "client")
	return cl
}

func createTopic(topic string, partitions int32) {
	cl := client()
	defer cl.Close()
	resp, err := kadm.NewClient(cl).CreateTopics(context.Background(), partitions, 1, nil, topic)
	must(err, "create topics")
	must(resp[topic].Err, "create "+topic)
}

func produce(cl *kgo.Client, topic string, partition int32, key, value string) error {
	r := &kgo.Record{Topic: topic, Partition: partition, Key: []byte(key), Value: []byte(value)}
	return cl.ProduceSync(context.Background(), r).FirstErr()
}

// reader is a consumer in a group that keeps what it reads and what it owns.
type reader struct {
	cl       *kgo.Client
	mu       sync.Mutex
	values   []string
	offsets  map[int32][]int64
	owned    map[int32]bool
	assigned int
	stop     chan struct{}
	done     chan struct{}
}

func newReader(group, topic string, opts ...kgo.Opt) *reader {
	r := &reader{offsets: map[int32][]int64{}, owned: map[int32]bool{}, stop: make(chan struct{}), done: make(chan struct{})}
	base := []kgo.Opt{
		kgo.ConsumerGroup(group),
		kgo.ConsumeTopics(topic),
		kgo.ConsumeResetOffset(kgo.NewOffset().AtStart()),
		kgo.SessionTimeout(10 * time.Second),
		kgo.HeartbeatInterval(500 * time.Millisecond),
		kgo.FetchMaxWait(200 * time.Millisecond),
		kgo.OnPartitionsAssigned(func(_ context.Context, _ *kgo.Client, m map[string][]int32) {
			r.mu.Lock()
			for _, p := range m[topic] {
				r.owned[p] = true
			}
			r.assigned++
			r.mu.Unlock()
		}),
		kgo.OnPartitionsRevoked(func(_ context.Context, _ *kgo.Client, m map[string][]int32) {
			r.mu.Lock()
			for _, p := range m[topic] {
				delete(r.owned, p)
			}
			r.mu.Unlock()
		}),
		kgo.OnPartitionsLost(func(_ context.Context, _ *kgo.Client, m map[string][]int32) {
			r.mu.Lock()
			for _, p := range m[topic] {
				delete(r.owned, p)
			}
			r.mu.Unlock()
		}),
	}
	r.cl = client(append(base, opts...)...)
	go func() {
		defer close(r.done)
		for {
			select {
			case <-r.stop:
				return
			default:
			}
			ctx, cancel := context.WithTimeout(context.Background(), 200*time.Millisecond)
			fetches := r.cl.PollFetches(ctx)
			cancel()
			fetches.EachRecord(func(rec *kgo.Record) {
				r.mu.Lock()
				r.values = append(r.values, string(rec.Value))
				r.offsets[rec.Partition] = append(r.offsets[rec.Partition], rec.Offset)
				r.mu.Unlock()
			})
		}
	}()
	return r
}

func (r *reader) count() int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return len(r.values)
}

func (r *reader) ownedCount() int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return len(r.owned)
}

func (r *reader) seen() []string {
	r.mu.Lock()
	defer r.mu.Unlock()
	return append([]string(nil), r.values...)
}

func (r *reader) close() {
	close(r.stop)
	<-r.done
	r.cl.Close()
}

func equal(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

func produceConsume() map[string]interface{} {
	createTopic("orders", 3)
	p := client(kgo.RecordPartitioner(kgo.StickyKeyPartitioner(nil)))
	for i := 0; i < 30; i++ {
		must(produce(p, "orders", -1, fmt.Sprintf("key-%d", i%7), fmt.Sprintf("v%d", i)), "produce")
	}
	p.Close()
	r := newReader("readers", "orders")
	until("30 records", 30*time.Second, func() bool { return r.count() >= 30 })
	must(r.cl.CommitUncommittedOffsets(context.Background()), "commit")
	r.close()
	for p, offsets := range r.offsets {
		for i, o := range offsets {
			check(o == int64(i), "partition %d offsets are not dense from 0: %v", p, offsets)
		}
	}
	return map[string]interface{}{"records": len(r.values), "partitions": len(r.offsets)}
}

func groupRebalance() map[string]interface{} {
	createTopic("work", 4)
	a := newReader("workers", "work")
	until("a owns 4", 30*time.Second, func() bool { return a.ownedCount() == 4 })
	b := newReader("workers", "work")
	until("a and b share", 30*time.Second, func() bool { return a.ownedCount() == 2 && b.ownedCount() == 2 })
	a.mu.Lock()
	b.mu.Lock()
	for p := range a.owned {
		check(!b.owned[p], "partition %d has two owners", p)
	}
	a.mu.Unlock()
	b.mu.Unlock()
	b.close()
	until("a owns 4 again", 30*time.Second, func() bool { return a.ownedCount() == 4 })
	a.close()
	return map[string]interface{}{"assignmentsOfA": a.assigned}
}

func idempotent() map[string]interface{} {
	createTopic("pay", 1)
	control("POST", "/v1/faults/duplicate-delivery", map[string]interface{}{"count": 2})
	p := client()
	for i := 0; i < 5; i++ {
		must(produce(p, "pay", 0, "k", fmt.Sprintf("idem-%d", i)), "idempotent produce")
	}
	p.Close()
	control("POST", "/v1/faults/duplicate-delivery", map[string]interface{}{"count": 1})
	plain := client(kgo.DisableIdempotentWrite())
	must(produce(plain, "pay", 0, "k", "plain"), "plain produce")
	plain.Close()
	r := newReader("audit", "pay")
	until("7 records", 30*time.Second, func() bool { return r.count() >= 7 })
	time.Sleep(300 * time.Millisecond)
	r.close()
	want := []string{"idem-0", "idem-1", "idem-2", "idem-3", "idem-4", "plain", "plain"}
	check(equal(r.seen(), want), "read %v, want %v", r.seen(), want)
	return map[string]interface{}{"values": r.seen()}
}

func transactions() map[string]interface{} {
	createTopic("ledger", 1)
	p := client(kgo.TransactionalID("ledger-writer"))
	ctx := context.Background()
	txn := func(values []string, commit bool) error {
		if err := p.BeginTransaction(); err != nil {
			return err
		}
		for _, v := range values {
			if err := produce(p, "ledger", 0, "acct", v); err != nil {
				return err
			}
		}
		end := kgo.TryAbort
		if commit {
			end = kgo.TryCommit
		}
		return p.EndTransaction(ctx, end)
	}
	must(txn([]string{"committed-0", "committed-1", "committed-2"}, true), "commit")
	must(txn([]string{"aborted-0", "aborted-1"}, false), "abort")
	must(txn([]string{"committed-3"}, true), "commit again")
	rc := newReader("rc", "ledger", kgo.FetchIsolationLevel(kgo.ReadCommitted()))
	ru := newReader("ru", "ledger", kgo.FetchIsolationLevel(kgo.ReadUncommitted()))
	until("records", 30*time.Second, func() bool { return rc.count() >= 4 && ru.count() >= 6 })
	time.Sleep(300 * time.Millisecond)
	rc.close()
	ru.close()
	want := []string{"committed-0", "committed-1", "committed-2", "committed-3"}
	check(equal(rc.seen(), want), "read committed %v, want %v", rc.seen(), want)
	check(len(ru.seen()) == 6, "read uncommitted %v", ru.seen())
	// A second instance with the same transactional id fences the first.
	fence := client(kgo.TransactionalID("ledger-writer"))
	must(fence.BeginTransaction(), "begin the fence")
	must(produce(fence, "ledger", 0, "acct", "fence"), "the fence produces")
	must(fence.EndTransaction(ctx, kgo.TryCommit), "the fence commits")
	fence.Close()
	err := txn([]string{"zombie"}, true)
	check(err != nil, "the zombie was not fenced")
	p.Close()
	return map[string]interface{}{"readCommitted": len(rc.seen()), "readUncommitted": len(ru.seen()), "fenced": err.Error()}
}

func faults() map[string]interface{} {
	createTopic("f", 1)
	p := client()
	must(produce(p, "f", 0, "k", "before"), "before")
	control("POST", "/v1/faults/request-latency", map[string]interface{}{"delay": "400ms", "api": "Produce"})
	t := time.Now()
	must(produce(p, "f", 0, "k", "slow"), "slow")
	check(time.Since(t) >= 400*time.Millisecond, "no latency: %v", time.Since(t))
	control("DELETE", "/v1/faults/request-latency", nil)
	control("POST", "/v1/faults/leader-change", map[string]interface{}{"topic": "f"})
	must(produce(p, "f", 0, "k", "after-leader-change"), "after leader change")
	control("POST", "/v1/faults/broker-unavailable", map[string]interface{}{"for": "5s"})
	done := make(chan error, 1)
	go func() { done <- produce(p, "f", 0, "k", "while-down") }()
	time.Sleep(time.Second)
	select {
	case err := <-done:
		fail("a produce completed while the broker was down: %v", err)
	default:
	}
	control("POST", "/v1/clock/advance", map[string]interface{}{"by": "5s"})
	must(<-done, "while down")
	p.Close()
	control("POST", "/v1/faults/consumer-lag", map[string]interface{}{"records": 1})
	r := newReader("lagging", "f")
	until("4 records", 30*time.Second, func() bool { return r.count() >= 4 })
	control("DELETE", "/v1/faults", nil)
	r.mu.Lock()
	before := r.assigned
	r.mu.Unlock()
	control("POST", "/v1/faults/rebalance-storm", map[string]interface{}{"rounds": 2})
	until("the storm", 40*time.Second, func() bool {
		_, f := control("GET", "/v1/faults", nil)
		active, _ := f["active"].([]interface{})
		return len(active) == 0
	})
	r.close()
	want := []string{"before", "slow", "after-leader-change", "while-down"}
	check(equal(r.seen(), want), "read %v, want %v", r.seen(), want)
	return map[string]interface{}{"assignedBeforeStorm": before, "assigned": r.assigned}
}

func force() map[string]interface{} {
	status, refused := control("POST", "/v1/state", map[string]interface{}{
		"topics":  []interface{}{map[string]interface{}{"name": "orders", "partitions": 1}},
		"records": []interface{}{map[string]interface{}{"topic": "orders", "partition": 0, "offset": 3, "records": []interface{}{map[string]interface{}{"value": "x"}}}},
	})
	check(status == 422, "an impossible state was not refused: %d", status)
	check(refused["law"] == "offsets in a partition are dense and increasing", "refused with %v", refused["law"])
	status, _ = control("POST", "/v1/state", map[string]interface{}{
		"topics": []interface{}{map[string]interface{}{"name": "orders", "partitions": 1}},
		"records": []interface{}{map[string]interface{}{"topic": "orders", "partition": 0, "records": []interface{}{
			map[string]interface{}{"value": "r0"}, map[string]interface{}{"value": "r1"},
			map[string]interface{}{"value": "r2"}, map[string]interface{}{"value": "r3"},
		}}},
		"committed": []interface{}{map[string]interface{}{"group": "resume", "topic": "orders", "partition": 0, "offset": 2}},
	})
	check(status == 200, "the state was not forced: %d", status)
	r := newReader("resume", "orders")
	until("2 records", 30*time.Second, func() bool { return r.count() >= 2 })
	time.Sleep(300 * time.Millisecond)
	r.close()
	check(equal(r.seen(), []string{"r2", "r3"}), "read %v", r.seen())
	return map[string]interface{}{"resumedAt": r.offsets[0][0]}
}

func main() {
	if len(os.Args) != 4 {
		fail("usage: franzgo-scenarios <scenario> <bootstrap> <admin url>")
	}
	scenario := os.Args[1]
	bootstrap, admin = os.Args[2], os.Args[3]
	scenarios := map[string]func() map[string]interface{}{
		"produce-consume": produceConsume,
		"group-rebalance": groupRebalance,
		"idempotent":      idempotent,
		"transactions":    transactions,
		"faults":          faults,
		"force":           force,
	}
	run, ok := scenarios[scenario]
	if !ok {
		names := []string{}
		for n := range scenarios {
			names = append(names, n)
		}
		sort.Strings(names)
		fail("unknown scenario %s; there are %v", scenario, names)
	}
	timer := time.AfterFunc(90*time.Second, func() { fail("scenario %s timed out", scenario) })
	summary := run()
	timer.Stop()
	summary["client"] = "franz-go"
	summary["scenario"] = scenario
	summary["ok"] = true
	out, err := json.Marshal(summary)
	if err != nil {
		fail("%v", errors.New("cannot write the summary"))
	}
	fmt.Println(string(out))
}
