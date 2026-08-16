# Connecting an ActiveMQ broker

The STOMP adapter dials the broker as a client and bridges a named list of topics in both directions. It speaks STOMP 1.2 over plain TCP. ActiveMQ Classic serves that protocol on port 61613 by default. Java CAL applications and this gateway can then sit on the same broker.

## Launch with Compose

The gateway and the broker are separate Compose files. Start a broker when one is needed:

```bash
docker compose -f compose/activemq.yml up -d
```

The broker console is at <http://127.0.0.1:8161/> (`admin` / `admin`). STOMP listens on `127.0.0.1:61613`.

The gateway Compose stack runs only `oa-gateway` and requires a configuration path:

```bash
OAG_CONFIG=$PWD/config/compose.toml docker compose -f compose/gateway.yml up --build
```

`config/compose.toml` is the container example: OWP on `0.0.0.0:9000`, STOMP off. To bridge a broker from the container, add `[stomp]` to a configuration of your own and set `broker` to a host the container can reach (for example `host.docker.internal:61613` when the broker is published on the Docker host). On the host itself, `config/asb.toml` and a local binary are the simpler path.

## Configure the adapter

`config/asb.toml` is the host-side example. Every STOMP key and its default is in [configuration.md](configuration.md). The section is:

```toml
[stomp]
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
| `login`, `passcode` | Left empty, both headers are omitted from `CONNECT`, which is what an unsecured broker expects. |
| `destination_prefix` | Joined to a topic name to make the STOMP destination. `/topic/` yields JMS topics on ActiveMQ Classic; `/queue/` works the same way for queues. Artemis or another broker with a different naming scheme is a change to this prefix. |
| `topics` | The bridge list. See the next section. |
| `unwrap_ma_payloads` | Publish the inner payload of an A-GRA wrapper alongside the wrapper itself, so subscribers can route on the payload rather than reading hex. |
| `reconnect` | Retry a lost session instead of stopping the adapter. |
| `max_frame_size` | Largest frame accepted from the broker, 16 MiB by default. It bounds both the read buffer and the `content-length` a peer can claim. |

## Topic mapping

Each entry in `topics` is bridged both ways. The adapter subscribes to `{destination_prefix}{topic}` on the broker and to the same name in the engine. An engine topic is therefore a JMS topic of the same name. For UCI traffic that name is the message type.

A topic that is not listed is not bridged in either direction. Publishing one from a WebSocket client is accepted by the protocol and then matches nothing. The gateway reports that once per route rather than dropping the message silently:

```
WARN nothing is subscribed to this route, so the publish went nowhere route=Foo adapter=owp
```

## Limits

- **The gateway does not terminate TLS and does not authenticate its own peers.** `login` and `passcode` are the broker's credentials and are sent in the clear. Keep both ends on loopback or a trusted segment. [SECURITY.md](../SECURITY.md) states the assumptions.
- **Heartbeats are off.** The gateway negotiates `heart-beat: 0,0`, so a dead TCP connection is noticed when a write fails rather than on a timer. A broker that requires heartbeats needs a change in `crates/oa-gateway-stomp/src/client.rs`.
- **Connect waits `connect_timeout_secs` (default 5) for TCP and for CONNECTED, each.**
- **Echo is suppressed when `suppress_echo` is true (the default).** Inbound MESSAGE is stamped `oag.origin_adapter`. Outbound SEND skips that id so a message does not loop between the gateway and the broker.
