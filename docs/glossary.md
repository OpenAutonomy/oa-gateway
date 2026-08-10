# Glossary

The gateway's own vocabulary, then the domain terms it borrows. Expansions below are the ones confirmable in the standards under `repos/`; where a term's expansion is not established there, the entry describes the role it plays in this repo instead of guessing.

## OA-Gateway's own terms

| Term | Meaning |
|---|---|
| **Envelope** | The unit that crosses the engine: id, route, string headers, content-type label, opaque `Bytes` payload. The engine reads only the route. |
| **RouteKey** | An address: `topic` plus an optional `type_hint`. |
| **topic** | The routing coordinate every envelope has. On the ActiveMQ path it equals the UCI message type name and the JMS/STOMP destination suffix. |
| **type_hint** | An optional discriminator within a topic — an OWP message name, a UCI message type. The engine only compares it for equality. `None` on a subscription means "every type on this topic". |
| **Adapter** | A protocol plugin owning its own I/O loop. Talks only to the engine, never to another adapter. |
| **Engine** | The router. Protocol-agnostic, payload-blind, in-process. |
| **wildcard subscription** | A subscription with `type_hint: None`, matching every type on its topic. |
| **conversion** | Mapping a payload between OMS JSON and UCI XML. Deliberately forgiving: an element the schema does not declare is carried rather than refused. |
| **validation** | Checking a payload against what the compiled schema states — declarations, occurrence ranges, alternations, abstract types. Separate from conversion, because a message can convert cleanly in both directions and still not be a valid instance of the standard. Controlled by `uci.validate`. |
| **violation** | One way a payload departs from the schema, with a dotted path to the element. A message is reported in full rather than at the first fault. |

## Domain terms

| Term | Meaning |
|---|---|
| **OMS** | Open Mission Systems. The standards family this gateway interoperates with; `repos/oms_standard`. |
| **UCI** | Universal Command and Control Interface. Supplies the message schema — message types such as `PositionReport` and `SubsystemStatus`; `repos/uci_standard`. `oa-gateway-uci` compiles the published XSD, so conversion covers whatever catalog the operator loads through `uci.schema`. |
| **CAL** | Critical Abstraction Layer. The OMS component boundary a participant implements. Java CALs speak OpenWire; `uci-cal-jms` and `sk-cal` are CAL implementations this gateway is meant to sit alongside. |
| **OWP** | OMS WebSocket Protocol. The text-frame protocol `oa-gateway-owp` serves, with `INIT`/`SUB`/`PUB`/`MSG`/`OK`/`ERR` operations. Its grammar comes from OMSC-SPC-013, the language-agnostic CAL specification. |
| **MT** | Message type. Used in this repo for the UCI type name carried in `type_hint`, as in "the wrapper MT and the inner MT". |
| **A-GRA** | The standard defining the `MA_RxDataPayload` and `MA_TxDataPayloadCommand` wrappers that `oa-gateway-agra` peels. Schema and interface volumes are in `repos/a-gra_standard` (`Schema/A-GRA_MessageDefinitions_v5_0_a.xsd`, and the ASK 5.0a volumes under `Documentation/`). |
| **MA-C2, MA-MA, MA-VI, MA-MS** | A-GRA interface designators. They line up with the ASK 5.0a interface volumes: Command and Control, Peer, Vehicle, and Mission Systems respectively. The first two are external interfaces and use the Rx/Tx hexBinary wrappers; the platform-facing two use native MTs and skip the wrapper. |
| **hexBinary** | `xs:hexBinary`, the XSD type A-GRA uses to carry a complete inner message as hex text inside a wrapper's `EncodedPayload`. |
| **PolySample** | A UCI construct whose JSON form carries a `$type` discriminator; `oa-gateway-uci` handles it explicitly. |
| **ASB** | In this repo, the ActiveMQ Classic broker acting as the shared bus between protocols — the setup `config/asb.toml` and `compose/activemq.yml` bring up. The "ASB path" is the naming rule where UCI message type, engine topic, STOMP destination, and JMS topic are all the same name. |

## Messaging protocols

| Term | Meaning |
|---|---|
| **STOMP** | Simple Text Oriented Messaging Protocol. A text framing over TCP that ActiveMQ accepts on `:61613`. `oa-gateway-stomp` is a STOMP *client*, not a JMS implementation. |
| **JMS** | Java Message Service. The Java messaging API whose topic model ActiveMQ exposes; a JMS topic `demo` is STOMP destination `/topic/demo`. |
| **OpenWire** | ActiveMQ's native binary wire protocol, and what Java CAL peers use. The gateway does not speak it — ActiveMQ bridges OpenWire and STOMP when the destination names match, which is why the naming rule matters. |
| **DDS** | Data Distribution Service. Another OMS transport, out of scope for v0. |
| **WebSocket** | The transport OWP runs over, on `ws://127.0.0.1:9000/` with subprotocol `owp`. |
