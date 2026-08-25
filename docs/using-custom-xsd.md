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

Two warnings are worth reading. Both mean the same thing: a constraint the gateway cannot read enforces nothing, and no later log can distinguish that from a value that passed.

```
WARN values of these types will not be checked beyond the facets on them primitives="xs:base64Binary"
WARN some schema patterns cannot be checked and will not be enforced count=1 types="TagType"
```

The first names a primitive with no check behind it, such as `xs:base64Binary` or `xs:anyURI`. `xs:string` is never listed, because there is nothing to check about a string beyond the facets on it. The second names a type whose `xs:pattern` uses a corner of XSD's regex language that this build does not translate: character-class subtraction, or the `\i` and `\c` name shorthands.

Neither warning fires on the published UCI catalog.

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

`validate` decides what a payload the schema does not permit costs. `warn` reports it and carries it; `reject` refuses it and tells the peer what was wrong; `off` skips the check. See [SECURITY.md](../SECURITY.md) for what validation covers and the two things it still does not.
