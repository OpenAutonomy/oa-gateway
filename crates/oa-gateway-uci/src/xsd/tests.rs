use super::compile;
use crate::schema::{MaxOccurs, Schema};
use crate::{Message, UciError};

fn wrap(body: &str) -> String {
    format!(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
                      xmlns:uci="urn:example"
                      targetNamespace="urn:example">{body}</xs:schema>"#
    )
}

fn compile_one(body: &str) -> Result<Schema, UciError> {
    compile(&[&wrap(body)])
}

#[test]
fn sequence_carries_names_types_and_occurrences() {
    let schema = compile_one(
        r#"<xs:complexType name="T">
             <xs:sequence>
               <xs:element name="a" type="xs:string"/>
               <xs:element name="b" type="xs:int" minOccurs="0"/>
               <xs:element name="c" type="xs:int" maxOccurs="unbounded"/>
               <xs:element name="d" type="xs:int" maxOccurs="8"/>
             </xs:sequence>
           </xs:complexType>"#,
    )
    .unwrap();

    let decls = schema.flatten("T").unwrap();
    let names: Vec<&str> = decls.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["a", "b", "c", "d"]);
    assert_eq!(decls[0].min_occurs, 1);
    assert_eq!(decls[1].min_occurs, 0);
    assert_eq!(decls[0].type_name, "xs:string");
    assert_eq!(decls[2].max_occurs, MaxOccurs::Unbounded);
    assert_eq!(decls[3].max_occurs, MaxOccurs::Bounded(8));
    assert!(decls[3].max_occurs.is_array());
    assert!(!decls[0].max_occurs.is_array());
}

#[test]
fn named_simple_types_resolve_through_their_chain() {
    let schema = compile_one(
        r#"<xs:simpleType name="Outer">
             <xs:restriction base="uci:Inner"/>
           </xs:simpleType>
           <xs:simpleType name="Inner">
             <xs:restriction base="xs:double">
               <xs:minInclusive value="0"/>
             </xs:restriction>
           </xs:simpleType>"#,
    )
    .unwrap();

    assert!(schema.is_simple("Outer"));
    assert!(schema.is_simple("Inner"));
    assert_eq!(schema.primitive("Outer"), "xs:double");
    assert_eq!(schema.primitive("Inner"), "xs:double");
    assert_eq!(schema.primitive("xs:int"), "xs:int");
}

#[test]
fn enumerations_reduce_to_their_base_primitive() {
    let schema = compile_one(
        r#"<xs:simpleType name="ClassificationEnum">
             <xs:restriction base="xs:string">
               <xs:enumeration value="U"/>
               <xs:enumeration value="S"/>
             </xs:restriction>
           </xs:simpleType>"#,
    )
    .unwrap();

    assert_eq!(schema.primitive("ClassificationEnum"), "xs:string");
}

#[test]
fn extension_flattens_base_before_extra() {
    let schema = compile_one(
        r#"<xs:complexType name="Base" abstract="true">
             <xs:sequence><xs:element name="kind" type="xs:string"/></xs:sequence>
           </xs:complexType>
           <xs:complexType name="Derived">
             <xs:complexContent>
               <xs:extension base="uci:Base">
                 <xs:sequence><xs:element name="extra" type="xs:int"/></xs:sequence>
               </xs:extension>
             </xs:complexContent>
           </xs:complexType>"#,
    )
    .unwrap();

    assert!(schema.complex_types["Base"].abstract_);
    assert!(!schema.complex_types["Derived"].abstract_);
    let names: Vec<&str> = schema
        .flatten("Derived")
        .unwrap()
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(names, ["kind", "extra"]);
}

#[test]
fn extension_without_a_compositor_inherits_only() {
    let schema = compile_one(
        r#"<xs:complexType name="Base">
             <xs:sequence><xs:element name="kind" type="xs:string"/></xs:sequence>
           </xs:complexType>
           <xs:complexType name="Derived">
             <xs:complexContent><xs:extension base="uci:Base"/></xs:complexContent>
           </xs:complexType>"#,
    )
    .unwrap();

    assert_eq!(schema.flatten("Derived").unwrap().len(), 1);
}

#[test]
fn choice_and_empty_content_are_represented() {
    let schema = compile_one(
        r#"<xs:complexType name="Pick">
             <xs:choice>
               <xs:element name="a" type="xs:string"/>
               <xs:element name="b" type="xs:string"/>
             </xs:choice>
           </xs:complexType>
           <xs:complexType name="Nothing"/>"#,
    )
    .unwrap();

    assert_eq!(
        schema.groups("Pick").unwrap()[0].kind,
        crate::schema::GroupKind::Choice,
        "an alternation must not compile to a sequence"
    );
    assert!(schema.groups("Nothing").unwrap().is_empty());
    assert!(schema.flatten("Nothing").unwrap().is_empty());
}

#[test]
fn primitives_are_canonical_whatever_prefix_the_document_uses() {
    let schema = compile(
        &[r#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema"
                                          xmlns:t="urn:example"
                                          targetNamespace="urn:example">
                               <xsd:element name="E" type="t:T"/>
                               <xsd:complexType name="T">
                                 <xsd:sequence>
                                   <xsd:element name="n" type="xsd:int"/>
                                 </xsd:sequence>
                               </xsd:complexType>
                             </xsd:schema>"#],
    )
    .unwrap();

    assert_eq!(schema.global_type("E"), Some("T"));
    assert_eq!(schema.flatten("T").unwrap()[0].type_name, "xs:int");
}

#[test]
fn documents_compose_across_an_include_boundary() {
    let a = wrap(
        r#"<xs:include schemaLocation="b.xsd"/>
           <xs:element name="Root" type="uci:RootType"/>
           <xs:complexType name="RootType">
             <xs:sequence><xs:element name="marking" type="uci:MarkingType"/></xs:sequence>
           </xs:complexType>"#,
    );
    let b = wrap(
        r#"<xs:complexType name="MarkingType">
             <xs:sequence><xs:element name="Classification" type="xs:string"/></xs:sequence>
           </xs:complexType>"#,
    );

    let schema = compile(&[&a, &b]).unwrap();
    assert!(schema.is_complex("MarkingType"));

    // The same reference is a hard error when the second document is absent.
    let err = compile(&[&a]).unwrap_err();
    assert!(
        matches!(&err, UciError::Xsd(m) if m.contains("MarkingType") && m.contains("missing")),
        "{err}"
    );
}

#[test]
fn unsupported_constructs_are_rejected_rather_than_ignored() {
    let cases = [
        (
            r#"<xs:complexType name="T">
                 <xs:sequence><xs:element name="a" type="xs:string"/></xs:sequence>
                 <xs:attribute name="id" type="xs:string"/>
               </xs:complexType>"#,
            "more than one content model",
        ),
        (
            r#"<xs:complexType name="T">
                 <xs:sequence>
                   <xs:choice><xs:element name="a" type="xs:string"/></xs:choice>
                 </xs:sequence>
               </xs:complexType>"#,
            "nests 'xs:choice'",
        ),
        (
            r#"<xs:complexType name="T">
                 <xs:complexContent>
                   <xs:restriction base="xs:anyType"/>
                 </xs:complexContent>
               </xs:complexType>"#,
            "only extension is supported",
        ),
        (
            r#"<xs:complexType name="T">
                 <xs:sequence><xs:element name="a"><xs:complexType/></xs:element></xs:sequence>
               </xs:complexType>"#,
            "anonymous inline types are not supported",
        ),
        (
            r#"<xs:simpleType name="T">
                 <xs:union memberTypes="xs:int xs:string"/>
               </xs:simpleType>"#,
            "only restriction is supported",
        ),
        (
            r#"<xs:complexType name="T">
                 <xs:sequence maxOccurs="unbounded">
                   <xs:element name="a" type="xs:string"/>
                 </xs:sequence>
               </xs:complexType>"#,
            "occurrence constraints on a compositor",
        ),
        (r#"<xs:group name="G"/>"#, "top-level 'xs:group'"),
    ];

    for (body, expected) in cases {
        let err = compile_one(body).unwrap_err();
        assert!(
            matches!(&err, UciError::Xsd(m) if m.contains(expected)),
            "expected {expected:?}, got {err}"
        );
    }
}

#[test]
fn redefinition_is_an_error() {
    let doc = wrap(r#"<xs:complexType name="T"/>"#);
    let err = compile(&[&doc, &doc]).unwrap_err();
    assert!(
        matches!(&err, UciError::Xsd(m) if m.contains("defined twice")),
        "{err}"
    );
}

#[test]
fn malformed_xml_is_reported_with_its_position() {
    let err = compile(&["<xs:schema>"]).unwrap_err();
    assert!(
        matches!(&err, UciError::Xsd(m) if m.contains("well-formed")),
        "{err}"
    );
}

#[test]
fn a_compiled_schema_drives_conversion_including_named_simple_types() {
    let schema = compile_one(
        r#"<xs:element name="Report" type="uci:ReportType"/>
           <xs:complexType name="ReportType">
             <xs:sequence>
               <xs:element name="Latitude" type="uci:LatitudeType"/>
               <xs:element name="Label" type="uci:LabelType" minOccurs="0"/>
               <xs:element name="Tag" type="xs:string" maxOccurs="unbounded"/>
             </xs:sequence>
           </xs:complexType>
           <xs:simpleType name="LatitudeType">
             <xs:restriction base="xs:double"/>
           </xs:simpleType>
           <xs:simpleType name="LabelType">
             <xs:restriction base="xs:string">
               <xs:enumeration value="alpha"/>
             </xs:restriction>
           </xs:simpleType>"#,
    )
    .unwrap();

    let src = r#"{"Report":{"Latitude":12.5,"Label":"alpha","Tag":["x","y"]}}"#;
    let xml = Message::from_json(src, &schema)
        .unwrap()
        .to_xml(&schema)
        .unwrap();
    assert!(xml.contains("<Latitude>12.5</Latitude>"), "{xml}");
    assert_eq!(xml.matches("<Tag>").count(), 2, "{xml}");

    let back = Message::from_xml(&xml, &schema).unwrap();
    let value: serde_json::Value = serde_json::from_str(&back.to_json(&schema).unwrap()).unwrap();

    // The point of the simple-type map: a named type restricting xs:double
    // has to come back as a JSON number, and one restricting xs:string as a
    // string, even though neither carries an xs: prefix in the schema.
    assert_eq!(
        value
            .pointer("/Report/Latitude")
            .and_then(serde_json::Value::as_f64),
        Some(12.5)
    );
    assert_eq!(
        value
            .pointer("/Report/Label")
            .and_then(serde_json::Value::as_str),
        Some("alpha")
    );
    assert_eq!(
        value
            .pointer("/Report/Tag")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
}
