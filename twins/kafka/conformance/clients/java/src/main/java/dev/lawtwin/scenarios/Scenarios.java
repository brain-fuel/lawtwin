package dev.lawtwin.scenarios;

import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Collection;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Properties;
import java.util.Set;
import java.util.TreeMap;
import java.util.concurrent.Future;
import java.util.function.BooleanSupplier;
import org.apache.kafka.clients.admin.Admin;
import org.apache.kafka.clients.admin.NewTopic;
import org.apache.kafka.clients.consumer.ConsumerRebalanceListener;
import org.apache.kafka.clients.consumer.ConsumerRecord;
import org.apache.kafka.clients.consumer.KafkaConsumer;
import org.apache.kafka.clients.producer.KafkaProducer;
import org.apache.kafka.clients.producer.ProducerRecord;
import org.apache.kafka.common.TopicPartition;
import org.apache.kafka.common.serialization.StringDeserializer;
import org.apache.kafka.common.serialization.StringSerializer;

/**
 * The Java client (kafka-clients) against the LawTwin Kafka twin. Each
 * scenario checks what the client observes and prints a JSON summary; the
 * Rust test that runs it then checks the contract's laws over the twin's
 * history.
 *
 * <p>[LAWTWIN_NS=&lt;prefix&gt;] java -cp ... dev.lawtwin.scenarios.Scenarios &lt;scenario&gt; &lt;bootstrap&gt; &lt;admin url&gt;
 */
public final class Scenarios {
  private static String bootstrap;
  private static String admin;
  private static final HttpClient HTTP = HttpClient.newHttpClient();

  /** A topic, group or transactional id, prefixed with LAWTWIN_NS so that scenarios share one real broker without meeting. */
  static String n(String name) {
    return System.getenv().getOrDefault("LAWTWIN_NS", "") + name;
  }

  private Scenarios() {}

  /** Calls the twin's admin API; gives the status and the body. */
  static String[] control(String method, String path, String json) throws Exception {
    HttpRequest.Builder b = HttpRequest.newBuilder(URI.create(admin + path)).header("content-type", "application/json");
    HttpRequest.BodyPublisher body = json == null ? HttpRequest.BodyPublishers.noBody() : HttpRequest.BodyPublishers.ofString(json);
    HttpResponse<String> r = HTTP.send(b.method(method, body).build(), HttpResponse.BodyHandlers.ofString());
    return new String[] {Integer.toString(r.statusCode()), r.body()};
  }

  static void check(boolean ok, String what) {
    if (!ok) {
      throw new AssertionError(what);
    }
  }

  static void until(String what, long millis, BooleanSupplier test) throws InterruptedException {
    long end = System.currentTimeMillis() + millis;
    while (!test.getAsBoolean()) {
      if (System.currentTimeMillis() > end) {
        throw new AssertionError("timed out waiting for " + what);
      }
      Thread.sleep(50);
    }
  }

  static Properties base() {
    Properties p = new Properties();
    p.put("bootstrap.servers", bootstrap);
    return p;
  }

  static void createTopic(String topic, int partitions) throws Exception {
    try (Admin a = Admin.create(base())) {
      a.createTopics(List.of(new NewTopic(topic, partitions, (short) 1))).all().get();
    }
  }

  static KafkaProducer<String, String> producer(Map<String, Object> extra) {
    Properties p = base();
    p.put("key.serializer", StringSerializer.class.getName());
    p.put("value.serializer", StringSerializer.class.getName());
    p.put("linger.ms", "0");
    p.put("delivery.timeout.ms", "30000");
    p.put("request.timeout.ms", "2000");
    p.put("retry.backoff.ms", "50");
    p.putAll(extra);
    return new KafkaProducer<>(p);
  }

  static void send(KafkaProducer<String, String> p, String topic, int partition, String value) throws Exception {
    p.send(new ProducerRecord<>(topic, partition, "k", value)).get();
  }

  /** A consumer in a group, polled by the caller, that keeps what it reads and owns. */
  static final class Reader implements ConsumerRebalanceListener {
    final KafkaConsumer<String, String> consumer;
    final List<String> values = new ArrayList<>();
    final Map<Integer, List<Long>> offsets = new TreeMap<>();
    final Set<Integer> owned = new HashSet<>();
    int assignments;

    Reader(String group, String topic, Map<String, Object> extra) {
      Properties p = base();
      p.put("group.id", group);
      p.put("group.protocol", "classic");
      p.put("key.deserializer", StringDeserializer.class.getName());
      p.put("value.deserializer", StringDeserializer.class.getName());
      p.put("auto.offset.reset", "earliest");
      p.put("enable.auto.commit", "false");
      p.put("session.timeout.ms", "10000");
      p.put("heartbeat.interval.ms", "500");
      p.put("fetch.max.wait.ms", "100");
      p.putAll(extra);
      consumer = new KafkaConsumer<>(p);
      consumer.subscribe(List.of(topic), this);
    }

    void poll() {
      for (ConsumerRecord<String, String> r : consumer.poll(Duration.ofMillis(100))) {
        values.add(r.value());
        offsets.computeIfAbsent(r.partition(), k -> new ArrayList<>()).add(r.offset());
      }
    }

    void pollUntil(String what, long millis, BooleanSupplier test) throws InterruptedException {
      long end = System.currentTimeMillis() + millis;
      while (!test.getAsBoolean()) {
        if (System.currentTimeMillis() > end) {
          throw new AssertionError("timed out waiting for " + what);
        }
        poll();
      }
    }

    @Override
    public void onPartitionsAssigned(Collection<TopicPartition> partitions) {
      for (TopicPartition tp : partitions) {
        owned.add(tp.partition());
      }
      assignments++;
    }

    @Override
    public void onPartitionsRevoked(Collection<TopicPartition> partitions) {
      for (TopicPartition tp : partitions) {
        owned.remove(tp.partition());
      }
    }

    @Override
    public void onPartitionsLost(Collection<TopicPartition> partitions) {
      onPartitionsRevoked(partitions);
    }
  }

  static String json(Map<String, Object> m) {
    StringBuilder b = new StringBuilder("{");
    for (Map.Entry<String, Object> e : m.entrySet()) {
      if (b.length() > 1) {
        b.append(',');
      }
      b.append('"').append(e.getKey()).append("\":");
      Object v = e.getValue();
      if (v instanceof String) {
        b.append('"').append(((String) v).replace("\"", "\\\"")).append('"');
      } else if (v instanceof List) {
        b.append('[');
        List<?> l = (List<?>) v;
        for (int i = 0; i < l.size(); i++) {
          b.append(i == 0 ? "" : ",").append('"').append(l.get(i)).append('"');
        }
        b.append(']');
      } else {
        b.append(v);
      }
    }
    return b.append('}').toString();
  }

  static Map<String, Object> produceConsume() throws Exception {
    createTopic(n("orders"), 3);
    try (KafkaProducer<String, String> p = producer(Map.of())) {
      for (int i = 0; i < 30; i++) {
        p.send(new ProducerRecord<>(n("orders"), "key-" + (i % 7), "v" + i)).get();
      }
    }
    Reader r = new Reader(n("readers"), n("orders"), Map.of());
    r.pollUntil("30 records", 30000, () -> r.values.size() >= 30);
    r.consumer.commitSync();
    r.consumer.close();
    for (List<Long> offsets : r.offsets.values()) {
      for (int i = 0; i < offsets.size(); i++) {
        check(offsets.get(i) == i, "offsets are dense from 0: " + offsets);
      }
    }
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("records", r.values.size());
    out.put("partitions", r.offsets.size());
    return out;
  }

  static Map<String, Object> groupRebalance() throws Exception {
    createTopic(n("work"), 4);
    Reader a = new Reader(n("workers"), n("work"), Map.of());
    a.pollUntil("a owns 4", 30000, () -> a.owned.size() == 4);
    Reader b = new Reader(n("workers"), n("work"), Map.of());
    long end = System.currentTimeMillis() + 30000;
    while (!(a.owned.size() == 2 && b.owned.size() == 2)) {
      check(System.currentTimeMillis() < end, "no split: " + a.owned + " and " + b.owned);
      a.poll();
      b.poll();
    }
    Set<Integer> both = new HashSet<>(a.owned);
    both.retainAll(b.owned);
    check(both.isEmpty(), "a partition has two owners: " + both);
    b.consumer.close();
    a.pollUntil("a owns 4 again", 30000, () -> a.owned.size() == 4);
    a.consumer.close();
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("assignmentsOfA", a.assignments);
    return out;
  }

  static Map<String, Object> idempotent() throws Exception {
    createTopic(n("pay"), 1);
    control("POST", "/v1/faults/duplicate-delivery", "{\"count\":2}");
    try (KafkaProducer<String, String> p = producer(Map.of("enable.idempotence", "true"))) {
      for (int i = 0; i < 5; i++) {
        send(p, n("pay"), 0, "idem-" + i);
      }
    }
    control("POST", "/v1/faults/duplicate-delivery", "{\"count\":1}");
    try (KafkaProducer<String, String> p = producer(Map.of("enable.idempotence", "false", "acks", "all"))) {
      send(p, n("pay"), 0, "plain");
    }
    Reader r = new Reader(n("audit"), n("pay"), Map.of());
    r.pollUntil("7 records", 30000, () -> r.values.size() >= 7);
    for (int i = 0; i < 3; i++) {
      r.poll();
    }
    r.consumer.close();
    List<String> want = List.of("idem-0", "idem-1", "idem-2", "idem-3", "idem-4", "plain", "plain");
    check(r.values.equals(want), "read " + r.values);
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("values", r.values);
    return out;
  }

  static Map<String, Object> transactions() throws Exception {
    createTopic(n("ledger"), 1);
    KafkaProducer<String, String> p = producer(Map.of("transactional.id", n("ledger-writer")));
    p.initTransactions();
    p.beginTransaction();
    for (int i = 0; i < 3; i++) {
      send(p, n("ledger"), 0, "committed-" + i);
    }
    p.commitTransaction();
    p.beginTransaction();
    for (int i = 0; i < 2; i++) {
      send(p, n("ledger"), 0, "aborted-" + i);
    }
    p.abortTransaction();
    p.beginTransaction();
    send(p, n("ledger"), 0, "committed-3");
    p.commitTransaction();
    Reader rc = new Reader(n("rc"), n("ledger"), Map.of("isolation.level", "read_committed"));
    Reader ru = new Reader(n("ru"), n("ledger"), Map.of("isolation.level", "read_uncommitted"));
    long end = System.currentTimeMillis() + 30000;
    while (rc.values.size() < 4 || ru.values.size() < 6) {
      check(System.currentTimeMillis() < end, "read " + rc.values + " and " + ru.values);
      rc.poll();
      ru.poll();
    }
    rc.consumer.close();
    ru.consumer.close();
    check(rc.values.equals(List.of("committed-0", "committed-1", "committed-2", "committed-3")), "read committed " + rc.values);
    check(ru.values.size() == 6, "read uncommitted " + ru.values);
    // A second instance with the same transactional id fences the first.
    try (KafkaProducer<String, String> fence = producer(Map.of("transactional.id", n("ledger-writer")))) {
      fence.initTransactions();
    }
    String fenced = "no";
    try {
      p.beginTransaction();
      Future<?> f = p.send(new ProducerRecord<>(n("ledger"), 0, "k", "zombie"));
      p.commitTransaction();
      f.get();
    } catch (Exception e) {
      fenced = e.getClass().getSimpleName();
    }
    check(!fenced.equals("no"), "the zombie was not fenced");
    p.close(Duration.ofSeconds(5));
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("readCommitted", rc.values.size());
    out.put("readUncommitted", ru.values.size());
    out.put("fenced", fenced);
    return out;
  }

  static Map<String, Object> faults() throws Exception {
    createTopic(n("f"), 1);
    KafkaProducer<String, String> p = producer(Map.of());
    send(p, n("f"), 0, "before");
    control("POST", "/v1/faults/request-latency", "{\"delay\":\"400ms\",\"api\":\"Produce\"}");
    long t = System.currentTimeMillis();
    send(p, n("f"), 0, "slow");
    check(System.currentTimeMillis() - t >= 400, "no latency");
    control("DELETE", "/v1/faults/request-latency", null);
    control("POST", "/v1/faults/leader-change", "{\"topic\":\"" + n("f") + "\"}");
    send(p, n("f"), 0, "after-leader-change");
    control("POST", "/v1/faults/broker-unavailable", "{\"for\":\"5s\"}");
    Future<?> down = p.send(new ProducerRecord<>(n("f"), 0, "k", "while-down"));
    Thread.sleep(1000);
    check(!down.isDone(), "a produce completed while the broker was down");
    control("POST", "/v1/clock/advance", "{\"by\":\"5s\"}");
    down.get();
    p.close();
    control("POST", "/v1/faults/consumer-lag", "{\"records\":1}");
    Reader r = new Reader(n("lagging"), n("f"), Map.of());
    r.pollUntil("4 records", 30000, () -> r.values.size() >= 4);
    r.consumer.commitSync();
    control("DELETE", "/v1/faults", null);
    int before = r.assignments;
    control("POST", "/v1/faults/rebalance-storm", "{\"rounds\":2}");
    r.pollUntil("the storm", 40000, () -> {
      try {
        return control("GET", "/v1/faults", null)[1].contains("\"active\":[]");
      } catch (Exception e) {
        return false;
      }
    });
    r.consumer.close();
    check(r.values.equals(List.of("before", "slow", "after-leader-change", "while-down")), "read " + r.values);
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("assignmentsBeforeStorm", before);
    out.put("assignments", r.assignments);
    return out;
  }

  static Map<String, Object> force() throws Exception {
    String[] refused = control("POST", "/v1/state", "{\"topics\":[{\"name\":\"a..b/c\",\"partitions\":1}]}");
    check(refused[0].equals("422"), "an impossible state was not refused: " + refused[0]);
    check(refused[1].contains("an invalid topic name is refused"), "refused with " + refused[1]);
    String[] forced = control("POST", "/v1/state",
        "{\"topics\":[{\"name\":\"" + n("orders") + "\",\"partitions\":1}],"
            + "\"records\":[{\"topic\":\"" + n("orders") + "\",\"partition\":0,\"records\":[{\"value\":\"r0\"},{\"value\":\"r1\"},{\"value\":\"r2\"},{\"value\":\"r3\"}]}],"
            + "\"committed\":[{\"group\":\"" + n("resume") + "\",\"topic\":\"" + n("orders") + "\",\"partition\":0,\"offset\":2}]}");
    check(forced[0].equals("200"), "the state was not forced: " + forced[1]);
    Reader r = new Reader(n("resume"), n("orders"), Map.of());
    r.pollUntil("2 records", 30000, () -> r.values.size() >= 2);
    r.consumer.close();
    check(r.values.equals(List.of("r2", "r3")), "read " + r.values);
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("resumedAt", r.offsets.get(0).get(0));
    return out;
  }

  /** Runs one scenario. */
  public static void main(String[] args) throws Exception {
    if (args.length != 3) {
      System.err.println("usage: Scenarios <scenario> <bootstrap> <admin url>");
      System.exit(2);
    }
    bootstrap = args[1];
    admin = args[2];
    Thread watchdog = new Thread(() -> {
      try {
        Thread.sleep(90000);
        System.err.println("scenario " + args[0] + " timed out");
        Runtime.getRuntime().halt(1);
      } catch (InterruptedException e) {
        return;
      }
    });
    watchdog.setDaemon(true);
    watchdog.start();
    Map<String, Object> summary;
    try {
      switch (args[0]) {
        case "produce-consume": summary = produceConsume(); break;
        case "group-rebalance": summary = groupRebalance(); break;
        case "idempotent": summary = idempotent(); break;
        case "transactions": summary = transactions(); break;
        case "faults": summary = faults(); break;
        case "force": summary = force(); break;
        default:
          System.err.println("unknown scenario " + args[0]);
          System.exit(2);
          return;
      }
    } catch (Throwable e) {
      e.printStackTrace();
      Runtime.getRuntime().halt(1);
      return;
    }
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("client", "java");
    out.put("scenario", args[0]);
    out.put("ok", true);
    out.putAll(summary);
    System.out.println(json(out));
    Runtime.getRuntime().halt(0);
  }
}
