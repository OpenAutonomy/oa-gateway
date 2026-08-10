# Connecting your ActiveMQ instance

The STOMP adapter dials your broker as a client and bridges a named list of topics in both directions. It speaks STOMP 1.2 over plain TCP (which ActiveMQ Classic serves on port 61613 by default). This allows applications using the Java CAL and this gateway to sit on the same broker.

## Launch with Compose

The gateway and the broker are separate Compose files. Start a broker when you need one:

```bash
docker compose -f compose/activemq.yml up -d
```

Console: <http://127.0.0.1:8161/> (`admin` / `admin`). STOMP: `127.0.0.1:61613`.

The gateway Compose stack only runs `oa-gateway`, and takes the config you give it:

```bash
docker compose -f compose/gateway.yml up --build
OAG_CONFIG=/path/to/my.toml docker compose -f compose/gateway.yml up --build
```

`config/compose.toml` is the default mount: OWP on `0.0.0.0:9000` inside the container, STOMP off. To bridge a broker from the container, enable `[stomp]` in a config of your own and set `broker` to a host the container can reach (for example `host.docker.internal:61613` when the broker is published on the Docker host). On the host itself, `config/asb.toml` and a local binary remain the simpler path.

## Configure the adapter

`config/asb.toml` is a worked example for a gateway on the host. The section is:

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

## Worth knowing

- **No TLS and no authentication of the gateway's own.** `login` and `passcode` are the broker's, sent in the clear. Keep both ends on loopback or a trusted segment; [SECURITY.md](../SECURITY.md) states the assumptions.
- **Heartbeats are off.** The gateway negotiates `heart-beat: 0,0`, so a dead TCP connection is noticed when a write fails rather than on a timer. A broker configured to require heartbeats will need that changed in `crates/oa-gateway-stomp/src/client.rs`.
- **Connecting takes at most five seconds.** The connect timeout is not configurable.
- **Echo is suppressed by convention.** Messages the adapter published are stamped `oag.origin_adapter`, and it skips deliveries carrying its own id, which is what keeps a message from looping between the gateway and the broker.
