# Configuration

The host takes one TOML file as its only argument. Every section in that file is optional. Naming `[loopback]`, `[owp]`, or `[stomp]` starts that adapter; omitting the table leaves it off. Set `enabled = false` to keep the keys in the file without starting the adapter. An unknown key is a startup error. At least one adapter table must be present and enabled.

```bash
./target/release/oa-gateway config/default.toml
```

Paths in the file are relative to the process working directory, normally the repository root. Hostnames (`owp.bind`, `stomp.broker`) are resolved once at startup. A name that does not resolve fails then, not later in a retry loop.

The gateway does not terminate TLS and does not authenticate its own peers. Bind loopback, or keep both ends on a trusted segment. [SECURITY.md](../SECURITY.md) states the assumptions.

## Shipped files

| File | Role |
|---|---|
| [`config/default.toml`](../config/default.toml) | Local development: loopback and OWP on loopback, STOMP off, no schema. |
| [`config/compose.toml`](../config/compose.toml) | Container example for `compose/gateway.yml`: OWP on `0.0.0.0:9000`, STOMP off. |
| [`config/asb.toml`](../config/asb.toml) | Host-side ActiveMQ bridge with a schema, `xml_baseline`, and STOMP enabled. Requires `scripts/fetch-uci-schema.sh` and a broker. |

`shipped_configs_parse` fails if one of those files names a key the structs do not declare.

## `[engine]`

The engine has no runtime settings of its own. This section controls how often the host logs engine counters. Omitting it uses a 30-second interval.

| Key | Default | What it does |
|---|---|---|
| `stats_interval_secs` | `30` | Seconds between `EngineStats` log lines (`published`, `delivered`, `dropped`). `0` disables the ticker. A line also warns when `dropped` increased since the last one. |

## `[uci]`

This section names the schema documents and what to do when a payload is not an instance of them. Conversion and validation both need the files listed explicitly; the standard is not redistributed here. See [using-custom-xsd.md](using-custom-xsd.md).

| Key | Default | What it does |
|---|---|---|
| `schema` | `[]` | XSD paths. Empty means no conversion and no validation. List every document the catalog spans; `xs:include` and `xs:import` are not followed. |
| `validate` | `"warn"` | `"warn"` reports a departure and carries the message; `"reject"` refuses it and tells the peer; `"off"` skips the check. Ignored when `schema` is empty. A typo is refused as `uci.validate: …`. |

`owp.xml_baseline` requires a schema. Startup refuses that combination rather than failing on the first converted message.

## `[loopback]`

Loopback is an in-process adapter with no socket. It is off unless this table is in the file.

| Key | Default | What it does |
|---|---|---|
| `enabled` | on when the section is present | `false` keeps the keys without starting the adapter. |
| `id` | `"loopback"` | Engine adapter id. |

## `[owp]`

OWP is the WebSocket server. It is off unless this table is in the file.

| Key | Default | What it does |
|---|---|---|
| `enabled` | on when the section is present | `false` keeps the keys without starting the adapter. |
| `id` | `"owp"` | Engine adapter id. One id covers every WebSocket; it is not a per-connection name. |
| `bind` | `"127.0.0.1:9000"` | Listen address, `host:port`. |
| `server_id` | `"oa-gateway-0"` | Identity sent on INIT. |
| `system_label` | `"OA-Gateway Prototype"` | Human-readable label sent on INIT. |
| `schema` | `"002.5.0"` | Protocol version string a client INIT must match exactly. This is not `[uci].schema`. Empty disables the check. |
| `unwrap_ma_payloads` | `true` | Peel A-GRA Rx/Tx hex wrappers on PUB and publish the wrapper and the inner message. |
| `xml_baseline` | `false` | Convert OMS JSON ↔ UCI XML at the socket so the engine and a broker see XML. Requires `[uci].schema`. |
| `max_frame_size` | `16777216` | Largest frame accepted from a client, in bytes. An oversized frame ends that session. |
| `max_connections` | `256` | Connections served at once. Further connections are closed on accept. |
| `max_subscriptions` | `1024` | Subscriptions one connection may hold. A SUB past the limit is refused and the session continues. |

A client is not authenticated, so these three limits isolate one peer from the memory of the others. The subscription default is larger than the UCI catalog, so a client can still subscribe to every message type in the standard.

## `[stomp]`

STOMP is a client toward an ActiveMQ or other broker. It is off unless this table is in the file, so a configuration that never names it does not need a broker. Worked examples and topic mapping are in [connecting-active-mq.md](connecting-active-mq.md).

| Key | Default | What it does |
|---|---|---|
| `enabled` | on when the section is present | `false` keeps the keys without starting the adapter. |
| `id` | `"stomp"` | Engine adapter id. |
| `broker` | `"127.0.0.1:61613"` | Broker address, `host:port`. |
| `host` | `"/"` | STOMP `host` header. ActiveMQ Classic typically wants `"/"`. |
| `login` | `""` | CONNECT login. Empty omits the header instead of sending a blank. |
| `passcode` | `""` | CONNECT passcode. Empty omits the header. Sent only when `login` is set. |
| `destination_prefix` | `"/topic/"` | Prepended to each topic to form a STOMP destination. |
| `topics` | `["demo"]` | Engine topic names and STOMP destination suffixes, bridged both ways. A name you do not list is not bridged. |
| `unwrap_ma_payloads` | `true` | Peel A-GRA Rx/Tx hex wrappers on inbound MESSAGE and publish the wrapper and the inner message. |
| `reconnect` | `true` | Retry the broker after a dropped session. |
| `reconnect_delay_secs` | `1` | Seconds to wait between reconnect attempts. |
| `connect_timeout_secs` | `5` | Seconds for TCP connect and for the CONNECTED wait, each. |
| `suppress_echo` | `true` | Skip outbound SEND when the envelope came from this adapter, so a message does not loop between the gateway and the broker. |
| `on_panic` | `"abort"` | `"abort"` ends the adapter when a session task panics; `"reconnect"` treats the panic as a failed session and then follows `reconnect`. A typo is refused as `stomp.on_panic: …`. |
| `max_frame_size` | `16777216` | Largest frame accepted from the broker, in bytes. It bounds both the read buffer and the `content-length` a peer can claim. |

`login` and `passcode` are the broker's credentials and are sent in the clear.

## Adding a section

A new adapter needs a `#[derive(Deserialize)]` struct in `crates/oa-gateway/src/config/` with `#[serde(deny_unknown_fields)]` and a default on every field, and a corresponding block in `config/default.toml`. [Writing an adapter](writing-an-adapter.md) covers the rest of the wiring.
