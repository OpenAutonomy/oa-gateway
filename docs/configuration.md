# Configuration

The host takes one TOML file as its only argument. Every section in that file is optional. Naming `[loopback]`, `[owp]`, `[stomp]`, or `[dds]` starts that adapter; omitting the table leaves it off. Set `enabled = false` to keep the keys in the file without starting the adapter. An unknown key is a startup error. At least one adapter table must be present and enabled.

```bash
./target/release/oa-gateway config/default.toml
```

Paths in the file are relative to the process working directory, normally the repository root. Hostnames (`owp.bind`, `stomp.broker`) are resolved once at startup. A name that does not resolve fails then, not later in a retry loop.

TLS is off unless configured. `owp.tls_cert` / `owp.tls_key` make the OWP listener serve `wss://`; `stomp.tls` makes the STOMP client dial the broker over TLS instead of plaintext. DDS has neither — its transport is UDP, not a TCP stream. A plain TLS connection is encrypted, not trusted; `owp.tls_client_ca` (a client authenticating to OWP) and `stomp.tls_client_cert`/`stomp.tls_client_key` (the gateway authenticating to a broker) are further, separate opt-in steps for that. There is still no authorization anywhere in this build — an authenticated peer is not treated any differently from one that was not. Bind loopback, or keep both ends on a trusted segment. [SECURITY.md](../SECURITY.md) states the assumptions.

## Shipped files

| File | Role |
|---|---|
| [`config/default.toml`](../config/default.toml) | Local development: loopback and OWP on loopback, STOMP off, no schema. |
| [`config/compose.toml`](../config/compose.toml) | Container example for `compose/gateway.yml`: OWP on `0.0.0.0:9000`, STOMP off. |
| [`config/asb.toml`](../config/asb.toml) | Host-side ActiveMQ bridge with a schema, `xml_baseline`, and STOMP enabled. Requires `scripts/fetch-uci-schema.sh` and a broker. |
| [`config/dds.toml`](../config/dds.toml) | Loopback plus a rustdds participant on domain 0. Requires [`config/dds-qos.xml`](../config/dds-qos.xml). |

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
| `tls_cert` | `""` | PEM certificate chain served to clients, leaf certificate first. Empty leaves the listener plaintext. Requires `tls_key`; setting one without the other is a startup error. |
| `tls_key` | `""` | PEM private key for `tls_cert`, in PKCS#8, PKCS#1, or SEC1 form. Empty leaves the listener plaintext. |
| `tls_client_ca` | `""` | PEM bundle of certificate authorities a client certificate must chain to. Empty accepts a client with or without one. Requires `tls_cert`/`tls_key`; a client that cannot present a certificate from this bundle is refused at the handshake. |
| `server_id` | `"oa-gateway-0"` | Identity sent on INIT. |
| `system_label` | `"OA-Gateway Prototype"` | Human-readable label sent on INIT. |
| `schema` | `"002.5.0"` | Protocol version string a client INIT must match exactly. This is not `[uci].schema`. Empty disables the check. |
| `unwrap_ma_payloads` | `true` | Peel A-GRA Rx/Tx hex wrappers on PUB and publish the wrapper and the inner message. |
| `xml_baseline` | `false` | Convert OMS JSON ↔ UCI XML at the socket so the engine and a broker see XML. Requires `[uci].schema`. |
| `max_frame_size` | `16777216` | Largest frame accepted from a client, in bytes. An oversized frame ends that session. |
| `max_connections` | `256` | Connections served at once. Further connections are closed on accept. |
| `max_subscriptions` | `1024` | Subscriptions one connection may hold. A SUB past the limit is refused and the session continues. |
| `init_timeout_secs` | `30` | Seconds from an accepted connection to a successful INIT before it is closed. Measured from the handshake, not reset by traffic. `0` disables. |
| `idle_timeout_secs` | `600` | Seconds with no frame in either direction on an active session before it is closed. Any client frame and any server frame (including a delivered MSG) resets it, so an active publisher or subscriber is never closed for being idle. `0` disables. |
| `allowed_origins` | `[]` | Exact `Origin` header values accepted at the WebSocket handshake. Empty accepts any origin, including none. A non-empty list refuses a handshake whose `Origin` is not one of these — a missing `Origin` included — with `403`. Match is verbatim: list every scheme, host, and port a browser client connects from. |
| `reconnect` | `false` | Rebind and accept again after the accept loop ends or panics, instead of leaving the adapter stopped until the whole process restarts. Defaults off so an existing deployment sees no behavior change until it opts in. |
| `reconnect_delay_secs` | `1` | Seconds to wait between rebind attempts. |
| `on_panic` | `"abort"` | `"abort"` ends the adapter when the accept loop panics; `"reconnect"` treats the panic as a failed session and then follows `reconnect`. A typo is refused as `owp.on_panic: …`. |

A client is not authenticated, so `max_frame_size`, `max_connections`, and `max_subscriptions` isolate one peer from the memory of the others, and `init_timeout_secs` / `idle_timeout_secs` keep a peer from holding a connection slot without making progress. The subscription default is larger than the UCI catalog, so a client can still subscribe to every message type in the standard.

With `tls_cert` and `tls_key` set, the listener speaks `wss://` and nothing else; a plaintext client is refused at the handshake. Set `tls_client_ca` too, and a client that cannot present a certificate from that bundle is refused as well — the one form of peer authentication this gateway has. It stops there: a client that connects with a valid certificate is not treated any differently from one that connected without `tls_client_ca` set at all. It may publish and subscribe exactly as before, and the gateway does not record or act on which certificate it was.

## `[stomp]`

STOMP is a client toward an ActiveMQ or other broker. It is off unless this table is in the file, so a configuration that never names it does not need a broker. Worked examples and topic mapping are in [connecting-active-mq.md](connecting-active-mq.md).

| Key | Default | What it does |
|---|---|---|
| `enabled` | on when the section is present | `false` keeps the keys without starting the adapter. |
| `id` | `"stomp"` | Engine adapter id. |
| `broker` | `"127.0.0.1:61613"` | Broker address, `host:port`. |
| `host` | `"/"` | STOMP `host` header. ActiveMQ Classic typically wants `"/"`. This is not `tls_server_name` — it is a protocol header, not a hostname. |
| `login` | `""` | CONNECT login. Empty omits the header instead of sending a blank. |
| `passcode` | `""` | CONNECT passcode. Empty omits the header. Sent only when `login` is set. |
| `destination_prefix` | `"/topic/"` | Prepended to each topic to form a STOMP destination. |
| `topics` | `["demo"]` | Engine topic names and STOMP destination suffixes, bridged both ways. A name you do not list is not bridged. |
| `unwrap_ma_payloads` | `true` | Peel A-GRA Rx/Tx hex wrappers on inbound MESSAGE and publish the wrapper and the inner message. |
| `reconnect` | `true` | Retry the broker after a dropped session. |
| `reconnect_delay_secs` | `1` | Seconds to wait between reconnect attempts. |
| `connect_timeout_secs` | `5` | Seconds for TCP connect, the TLS handshake when `tls` is set, and the CONNECTED wait, each. |
| `suppress_echo` | `true` | Skip outbound SEND when the envelope came from this adapter, so a message does not loop between the gateway and the broker. |
| `on_panic` | `"abort"` | `"abort"` ends the adapter when a session task panics; `"reconnect"` treats the panic as a failed session and then follows `reconnect`. A typo is refused as `stomp.on_panic: …`. |
| `max_frame_size` | `16777216` | Largest frame accepted from the broker, in bytes. It bounds both the read buffer and the `content-length` a peer can claim. |
| `tls` | `false` | Wrap the broker connection in TLS. ActiveMQ Classic's SSL transport connector conventionally listens on `61612`, not `61613`. |
| `tls_ca` | `""` | PEM bundle of the certificate authorities the broker's certificate must chain to. Empty uses the operating system trust store, which is where an organizational CA normally lives. |
| `tls_server_name` | `""` | Name checked against the broker's certificate. Empty uses the host part of `broker`; a bare IP address there requires an IP SAN in the certificate, which most do not have. |
| `tls_client_cert` | `""` | PEM certificate chain presented to the broker, leaf certificate first. Empty presents nothing. Requires `tls_client_key` and `tls = true`. |
| `tls_client_key` | `""` | PEM private key for `tls_client_cert`, in PKCS#8, PKCS#1, or SEC1 form. Empty presents nothing. |

`login` and `passcode` are the broker's credentials. They are sent in the clear unless `tls` is on, which is the reason to turn it on. `tls_client_cert`/`tls_client_key` are a separate, further step: presenting a certificate to a broker whose SSL transport connector requires one (ActiveMQ's `needClientAuth`), independent of whatever `login`/`passcode` also send.

## `[dds]`

DDS is a participant on a domain. It is off unless this table is in the file. There is no broker hostname to resolve. Worked examples, the QoS subset, and topic mapping are in [connecting-dds.md](connecting-dds.md).

| Key | Default | What it does |
|---|---|---|
| `enabled` | on when the section is present | `false` keeps the keys without starting the adapter. |
| `id` | `"dds"` | Engine adapter id. |
| `provider` | `"rustdds"` | Which `DdsProvider` to construct. `"rustdds"` is the only legal value in this build. A typo is refused as `dds.provider: …`. |
| `domain_id` | `0` | DDS domain the participant joins. Peers must use the same id. |
| `qos` | required | Path to a QoS file. Missing or empty is a startup error when the section is present. rustdds parses a documented DDS-XML subset (reliability, durability, history). A later vendor provider may pass the same path to its own loader. |
| `topics` | `["demo"]` | Engine topic names and DDS topic names, bridged both ways. A name you do not list is not bridged. |
| `unwrap_ma_payloads` | `true` | Peel A-GRA Rx/Tx wrappers on inbound samples and publish the wrapper and the inner message. |
| `suppress_echo` | `true` | Skip outbound writes when the envelope came from this adapter, so a message does not loop between the gateway and the domain. |
| `reconnect` | `false` | Rejoin the domain after the session ends or panics, instead of leaving the adapter stopped until the whole process restarts. Defaults off so an existing deployment sees no behavior change until it opts in. |
| `reconnect_delay_secs` | `1` | Seconds to wait between rejoin attempts. |
| `on_panic` | `"abort"` | `"abort"` ends the adapter when the session panics; `"reconnect"` treats the panic as a failed session and then follows `reconnect`. A typo is refused as `dds.on_panic: …`. |
| `max_sample_size` | `16777216` | Largest inbound sample accepted, in bytes, before it is unwrapped or converted. An oversized sample is dropped and logged rather than ending the session — DDS has no per-peer connection to end. |

Inbound samples are checked against `[uci].schema` the same way OWP traffic is; `[uci].validate` decides what a violation costs, and has no effect without a schema. Unlike OWP, DDS has no peer connection to notify, so `reject` drops the sample and logs it rather than answering an error frame.

`[dds]` is omitted from [`config/default.toml`](../config/default.toml) because the local toy has no domain to join. Use [`config/dds.toml`](../config/dds.toml) when you want one.

## Adding a section

A new adapter needs a `#[derive(Deserialize)]` struct in `crates/oa-gateway/src/config/` with `#[serde(deny_unknown_fields)]` and a default on every field, and a corresponding block in a shipped example (`config/default.toml`, or a worked file such as `config/dds.toml` when the adapter is opt-in). [Writing an adapter](writing-an-adapter.md) covers the rest of the wiring.
