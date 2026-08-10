# Connecting your ActiveMQ instance

The STOMP adapter dials your broker as a client — the gateway never listens for one — and bridges a named list of topics in both directions. It speaks STOMP 1.2 over plain TCP, which ActiveMQ Classic serves on port 61613 alongside OpenWire on 61616, so a Java CAL and the gateway can sit on the same broker.

## A broker to try it against

If you do not already have one, `compose/activemq.yml` starts ActiveMQ Classic with the ports the examples use:

```bash
docker compose -f compose/activemq.yml up -d
```

The web console is on <http://localhost:8161> with `admin` / `admin`, which is a quick way to publish a message by hand and watch it cross.

## Configure the adapter

`config/asb.toml` is a worked example. The section is:

```toml
[stomp]
enabled = true
id = "stomp"
broker = "127.0.0.1:61613"
host = "/"
login = ""
passcode = ""
destination_prefix = "/topic/"
topics = ["PositionReport"]
unwrap_ma_payloads = true
reconnect = true
```

| Key | What it does |
|---|---|
| `broker` | `host:port` of the broker's STOMP listener. Hostnames are resolved once at startup, so a name that does not resolve is a startup error rather than a silent retry loop. |
| `host` | The STOMP `host` header. ActiveMQ Classic wants `/`. |
| `login`, `passcode` | Left empty, both headers are omitted from `CONNECT` entirely, which is what an unsecured broker expects. |
| `destination_prefix` | Joined to a topic name to make the STOMP destination. `/topic/` gives you JMS topics on ActiveMQ Classic; `/queue/` works the same way for queues, and Artemis or another broker with a different naming scheme is a matter of changing this. |
| `topics` | The bridge list; see below. |
| `unwrap_ma_payloads` | Publish the inner payload of an A-GRA wrapper alongside the wrapper itself, so subscribers can route on the payload rather than reading hex. |
| `reconnect` | Retry a lost session instead of stopping the adapter. |
| `max_frame_size` | Largest frame accepted from the broker, 16 MiB by default. It bounds both the read buffer and the `content-length` a peer can claim. |

## How topics map

Each entry in `topics` is bridged both ways: the adapter subscribes to `{destination_prefix}{topic}` on the broker, and subscribes in the engine to the same name. An engine topic is therefore a JMS topic of the same name, and for UCI traffic that name is the message type.

A topic you do not list is not bridged in either direction. Publishing one from a WebSocket client is accepted by the protocol and then matches nothing, so rather than let the message disappear the gateway says so once per route:

```
WARN nothing is subscribed to this route, so the publish went nowhere route=Foo adapter=owp
```

## What a good start looks like

```
INFO oa_gateway: starting stomp adapter id=stomp broker=127.0.0.1:61613
INFO oa_gateway_stomp::adapter: stomp connected adapter=stomp broker=127.0.0.1:61613 topics=["PositionReport"]
```

If the broker is not up yet, or goes away later, the adapter says so and retries every second, keeping its place in the config:

```
WARN oa_gateway_stomp::adapter: stomp session failed, retrying adapter=stomp error=...
```

Each new session clears the adapter's previous engine subscriptions before making new ones, so a reconnect does not leave stale routes behind. Set `reconnect = false` if you would rather a broker outage be fatal.

## Checking it end to end

With `config/asb.toml` running, send a UCI XML message to `/topic/PositionReport` — from the web console, or any STOMP client — and a WebSocket subscriber on the other side of the gateway receives it as OMS JSON, because `owp.xml_baseline` is on in that config:

```
<- MSG s1 {"PositionReport": {"MessageHeader": {"Mode": "SIMULATION", ...}}}
```

Validation runs on the way out, so a payload that converts but does not comply is reported once per subscription rather than per message:

```
WARN delivered payload does not follow the UCI schema; forwarding it anyway, and later
     ones on this subscription are not logged adapter=owp sid=s1
     violations=PositionReport: 'SecurityInformation' is required and absent; …
```

The same round trip runs as a test. `scripts/live-activemq.sh` brings the compose broker up, waits for the STOMP port, and runs the ignored live test against it; point it at a broker of your own with `OAG_ACTIVEMQ_STOMP=host:port`. For tests that should not need Docker at all, `oa-gateway-testing` has `start_mini_broker()`, an in-process STOMP broker on an ephemeral port — `crates/oa-gateway-testing/tests/stomp_bridge.rs` is the pattern to copy.

## Worth knowing

- **No TLS and no authentication of the gateway's own.** `login` and `passcode` are the broker's, sent in the clear. Keep both ends on loopback or a trusted segment; [SECURITY.md](../SECURITY.md) states the assumptions.
- **Heartbeats are off.** The gateway negotiates `heart-beat: 0,0`, so a dead TCP connection is noticed when a write fails rather than on a timer. A broker configured to require heartbeats will need that changed in `crates/oa-gateway-stomp/src/client.rs`.
- **Connecting takes at most five seconds.** The connect timeout is not configurable.
- **Echo is suppressed by convention.** Messages the adapter published are stamped `oag.origin_adapter`, and it skips deliveries carrying its own id, which is what keeps a message from looping between the gateway and the broker.
