# Connecting a DDS domain

The DDS adapter joins a domain as a participant and bridges a named list of topics in both directions. The engine topic and the DDS topic are the same name. Samples are A-GRA Rx/Tx only: `MA_RxDataPayload` and `MA_TxDataPayloadCommand` fields plus the inner UCI bytes. The adapter does not generate the UCI catalog as IDL types.

The first provider is rustdds (Apache-2.0). The adapter never names rustdds types. A later Cyclone DDS or Fast DDS implementation is another type behind the same `DdsProvider` trait; it is not in this build.

## Launch

There is no broker to start. Two rustdds participants on the same `domain_id` discover each other. The shipped example is loopback plus one participant:

```bash
./target/release/oa-gateway config/dds.toml
```

`config/dds.toml` is omitted from the default local toy because that process has no domain to join. Add `[dds]` to a configuration of your own when a peer will be on the domain.

## Configure the adapter

Every DDS key and its default is in [configuration.md](configuration.md). The section is:

```toml
[dds]
id = "dds"
provider = "rustdds"
domain_id = 0
qos = "config/dds-qos.xml"
topics = ["demo"]
unwrap_ma_payloads = true
suppress_echo = true
```

| Key | What it does |
|---|---|
| `provider` | Which stack constructs the participant. `"rustdds"` is the only legal value now. An unknown name is a startup error. |
| `domain_id` | The DDS domain. Peers that should see these topics must use the same id. |
| `qos` | Path to a QoS file. Required when the section is present. A missing file fails at startup. |
| `topics` | The bridge list. See the next section. |
| `unwrap_ma_payloads` | Publish the inner payload of an A-GRA wrapper alongside the wrapper itself, so subscribers can route on the payload rather than reading hex. |
| `suppress_echo` | Skip outbound writes that originated on this adapter, so a message does not loop between the gateway and the domain. |

## Topic mapping

Each entry in `topics` is bridged both ways. The adapter creates a DDS topic of that name and a wildcard engine subscription on the same name. A topic that is not listed is not bridged in either direction.

DDS allows one type per topic name. The on-wire type is therefore a single struct, `MaDataPayload`, with a `kind` of `"rx"` or `"tx"`. `encoded` is the inner UCI payload as octets, not hex text. The type name passed to rustdds is `"MaDataPayload"`.

## QoS file

`[dds] qos` is a file path, not a set of TOML knobs. The adapter opens it only to fail startup if the file is missing. Interpretation is the provider's job.

rustdds has no vendor XML loader, so the first provider parses a documented DDS-XML subset into reliability, durability, and history. Unknown elements are refused. The shipped profile is [`config/dds-qos.xml`](../config/dds-qos.xml): reliable, volatile, keep-last 16.

Allowed elements are `dds`, `qos_library`, `qos_profile`, `datawriter_qos`, `datareader_qos`, `reliability`, `durability`, `history`, `kind`, and `depth`. Reliability kinds are `RELIABLE` and `BEST_EFFORT`. Durability kinds are `VOLATILE` and `TRANSIENT_LOCAL`. History kinds are `KEEP_LAST` (with `depth`) and `KEEP_ALL`.

A later FFI provider may pass the same path straight into that library and ignore this subset parser. Partitions, per-topic QoS, and DDS Security are out of scope for this adapter.

## Echo

Inbound samples are stamped `oag.origin_adapter`. When `suppress_echo` is true (the default), outbound writes skip that id. The rustdds provider also drops samples whose writer shares this participant's GUID prefix, because rustdds delivers local writes to the local reader. The engine does not skip by origin.

## Who this talks to

This build talks to another rustdds participant, including a second OA-Gateway process that names the same `domain_id` and topic list. A vendor CAL that already speaks DDS is the next interoperability target; it is not covered by the in-process tests. Those tests start a second rustdds participant in the same process and do not require Docker or a system DDS library.

## Limits

- **DDS traffic is not encrypted and peers are not authenticated.** RTPS runs on UDP, so the TLS available to the OWP adapter does not apply here; the DDS equivalent is DDS Security, which this build does not configure. Keep participants on a trusted segment. [SECURITY.md](../SECURITY.md) states the assumptions.
- **One QoS profile applies to every topic** the adapter creates. There is no per-topic override in the TOML.
- **Volatile durability** means a write before discovery is lost. A peer that joins late will not see earlier samples unless the QoS file uses `TRANSIENT_LOCAL`.
- **No native UCI IDL catalog.** Inner messages stay bytes inside `MaDataPayload.encoded`.
