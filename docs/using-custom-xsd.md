# Using a custom XSD

Nothing about a message set is compiled into the binary. The gateway compiles
the XSD you name at startup and validates UCI XML against it. A
program-specific Message Set, a later UCI revision, or a trimmed subset all
work the same way. `scripts/fetch-uci-schema.sh` is only a convenience for
the published UCI 2.5 documents.

`[uci]` keys and defaults are also listed in [configuration.md](configuration.md).

## Configuration

```toml
[uci]
schema = [
  "/path/to/YourMessageDefinitions.xsd",
  "/path/to/YourSecurityMarkings.xsd",
]
validate = "warn"
```

A schema is optional. Without one, payloads cross the engine unchecked.

Only the *first* entry is actually read: libxml2 resolves `xs:include`/
`xs:import` itself, relative to that file's own path on disk, so the first
entry has to be the document that spans the rest of the catalog
(`...MessageDefinitions...xsd` before `...SecurityMarkings...xsd`, say). The
published UCI and A-GRA catalogs both work this way already. List any
further files anyway if you like — the gateway warns at startup when
`[uci].schema` names more than two, since a larger custom Message Set could
have a document the first entry does not transitively include, which would
then go unchecked silently.

## Startup output

A schema that will not compile stops the gateway and names the reason.
Otherwise the log reports that validation is on and in which mode:

```
INFO oa_gateway::serve: schema validation mode=warn
```

## Validation

`validate` decides what a payload the schema does not permit costs. `warn`
reports it and carries it; `reject` refuses it and tells the peer what was
wrong; `off` skips the check. See [SECURITY.md](../SECURITY.md) for what
validation covers and the one thing it still does not.

Validation is done entirely by `oa-gateway-xsd-libxml` against libxml2 — the
same schema compile the gateway runs at startup, reused for every payload on
a dedicated thread (`SchemaValidationContext` is `!Send`). There is no
separate, hand-rolled schema model: whatever libxml2 accepts as valid XSD,
the gateway accepts.

## Checking a schema before deploy

The simplest check is to run the gateway itself against your schema and a
sample payload — a schema that does not compile fails startup immediately,
named by file. To check compilation alone, without starting any adapter,
`oa_gateway_xsd_libxml::XsdLibxmlValidator::compile` is the same call the
host makes:

```rust
let validator = oa_gateway_xsd_libxml::XsdLibxmlValidator::compile(
    "/path/to/YourMessageDefinitions.xsd".as_ref(),
)?;
```

`crates/oa-gateway-xsd-libxml/tests/agra_schema.rs` is a worked example:
it compiles the real A-GRA 5.0a schema and validates XML fixtures against
it, including a path (`AGRA_SCHEMA_DIR`) for pointing the same test at a
different copy of the schema.
