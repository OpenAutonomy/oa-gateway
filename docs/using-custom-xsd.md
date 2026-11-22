# Using a custom XSD

Nothing about a message set is compiled into the binary. The gateway reads XSD at startup and builds its schema from the documents you name. A program-specific Message Set, a later UCI revision, or a trimmed subset all work the same way. `scripts/fetch-uci-schema.sh` is only a convenience for the published UCI 2.5 documents.

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

Pass every document the schema spans. `xs:include` and `xs:import` are not followed, because following them would mean reading file paths out of the schema text. A document you leave out appears as unresolved type names, not as a type that quietly goes missing later.

A schema is optional. Without one, payloads cross the engine untouched. `owp.xml_baseline` requires a schema, because converting between OMS JSON and XML is what the schema is for.

## Startup output

A schema that will not compile stops the gateway and names the reason. Otherwise the log reports what was read:

```
INFO oa_gateway: uci schema compiled files=1 messages=1 complex_types=1 simple_types=1
```

This is the conversion schema only. Validation is a separate compile, by
`oa-gateway-xsd-libxml`, of the same `[uci].schema` entry file against
libxml2 — see [Validation](#validation) below.

## Supported XSD subset

The compiler follows UCI's Schema Style & Design Specification rather than XSD at large. It refuses what falls outside that subset instead of guessing. A custom schema needs to keep to:

- **Named top-level types.** An element with an anonymous inline type is refused. Give the type a name and refer to it.
- **Extension as the only complex derivation.** `xs:restriction` on complex content is refused, as is `xs:redefine`.
- **Flat compositors.** An `xs:sequence` or `xs:choice` that holds another compositor, or that carries its own `minOccurs`/`maxOccurs`, has no representation in the flat element model.
- **Restriction for simple types**, with the facets the validator understands: `enumeration`, `pattern`, `length`, `minLength`, `maxLength`, and the four `min`/`max` bounds. `whiteSpace` is accepted and ignored. Any other facet is refused, because a constraint that is silently dropped cannot be told apart from one that was never there.

The constructs outside that list do not appear in UCI, so none of them is a limitation you will meet with the published schema. Attributes and `xs:any` are refused by name. `substitutionGroup=` is the one construct read past rather than refused: the element still compiles, but the substitution relationship is not known, so nothing enforces it. If a message set needs any of those constructs, the compiler is `crates/oa-gateway-uci/src/xsd/`, and the error names the construct it stopped on.

## Checking a schema before deploy

The suite has a test that compiles a real schema and walks every type reference. Point it at a custom catalog:

```bash
OAG_UCI_XSD=/path/YourDefs.xsd:/path/YourMarkings.xsd \
  cargo test -p oa-gateway-uci -- --ignored --nocapture
```

It reports what compiled, how long it took, and the deepest message the catalog can express. That depth has to stay under the conversion limit, or nesting the schema permits will not convert.

## Validation

`validate` decides what a payload the schema does not permit costs. `warn` reports it and carries it; `reject` refuses it and tells the peer what was wrong; `off` skips the check. See [SECURITY.md](../SECURITY.md) for what validation covers and the one thing it still does not.

Validation itself is a second, independent compile, by `oa-gateway-xsd-libxml`
against libxml2 — not the `Schema` conversion reads and writes XML with. The
two compiles want their input named differently, and this matters once a
custom schema spans more than one file:

- **Conversion** (`[uci].schema`) wants every document the schema spans,
  listed explicitly, because it does not follow `xs:include`/`xs:import` (see
  above).
- **Validation** wants one entry point and resolves `xs:include`/
  `xs:import` itself, relative to that file's path on disk. It compiles
  `[uci].schema`'s *first* entry — so put the document that includes the
  others first in the list. The published UCI and A-GRA catalogs both work
  this way already: a single `...MessageDefinitions...xsd` that includes a
  `...SecurityMarkings...xsd` from the same directory.

The gateway warns at startup if `[uci].schema` lists more than two files,
since a larger custom Message Set could have a document that the first
entry does not transitively include — which would silently go unchecked on
the validation side while still converting normally.
