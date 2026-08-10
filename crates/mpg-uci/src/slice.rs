//! Hand-authored UCI 2.5 *slice* — not the full message catalog.
//!
//! Covers Ping, PositionReport (mpg-testing fixtures), PolySample (`$type`),
//! and MA_RxDataPayload / MA_TxDataPayloadCommand enough to convert wrap shells.

use std::sync::OnceLock;

use crate::schema::{el, el_many, el_opt, Schema};

/// Shared slice used by OWP when `xml_baseline` is on.
#[must_use]
pub fn v25() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(build_v25)
}

fn build_v25() -> Schema {
    let mut s = Schema::new();

    s.complex(
        "PingType",
        vec![el_opt("n", "xs:int"), el_opt("from", "xs:string")],
    )
    .element("Ping", "PingType");

    s.complex("SystemIDType", vec![el("UUID", "xs:string")])
        .complex(
            "OwnerProducerType",
            vec![el("GovernmentIdentifier", "xs:string")],
        )
        .complex(
            "SecurityInformationType",
            vec![
                el("Classification", "xs:string"),
                el_many("OwnerProducer", "OwnerProducerType"),
            ],
        )
        .complex(
            "MessageHeaderType",
            vec![
                el("SystemID", "SystemIDType"),
                el("Timestamp", "xs:dateTime"),
                el("SchemaVersion", "xs:string"),
                el("Mode", "xs:string"),
            ],
        )
        .complex(
            "PositionType",
            vec![
                el("Latitude", "xs:double"),
                el("Longitude", "xs:double"),
                el_opt("Altitude", "xs:double"),
                el_opt("Timestamp", "xs:dateTime"),
            ],
        )
        .complex("InertialStateType", vec![el("Position", "PositionType")])
        .complex(
            "MessageDataType",
            vec![
                el_opt("n", "xs:int"),
                el_opt("SystemID", "SystemIDType"),
                el_opt("Source", "xs:string"),
                el_opt("CurrentOperatingDomain", "xs:string"),
                el_opt("InertialState", "InertialStateType"),
            ],
        )
        .complex(
            "PositionReportType",
            vec![
                el_opt("SecurityInformation", "SecurityInformationType"),
                el_opt("MessageHeader", "MessageHeaderType"),
                el_opt("MessageData", "MessageDataType"),
            ],
        )
        .element("PositionReport", "PositionReportType");

    s.complex_abstract("DetailBase", vec![el("kind", "xs:string")])
        .extend(
            "InertialDetail",
            "DetailBase",
            vec![el("Latitude", "xs:double"), el("Longitude", "xs:double")],
        )
        .complex("PolySampleType", vec![el("Detail", "DetailBase")])
        .element("PolySample", "PolySampleType");

    s.complex(
        "PriorityType",
        vec![
            el_opt("Priority", "xs:int"),
            el_opt("PrecedenceWithinPriority", "xs:int"),
        ],
    )
    .complex("UuidRefType", vec![el("UUID", "xs:string")])
    .complex(
        "RxMessageDataType",
        vec![
            el_opt("RxDataPayloadID", "UuidRefType"),
            el_opt("DataPayloadOriginatorID", "UuidRefType"),
            el_opt("CommandID", "UuidRefType"),
            el_opt("CommandState", "xs:string"),
            el_opt("EncodedPayload", "xs:hexBinary"),
            el_opt("Timestamp", "xs:dateTime"),
            el_opt("MessageType", "xs:string"),
            el_opt("DestinationRouting", "xs:string"),
            el_opt("Priority", "PriorityType"),
        ],
    )
    .complex(
        "MaWrapperType",
        vec![
            el_opt("SecurityInformation", "SecurityInformationType"),
            el_opt("MessageHeader", "MessageHeaderType"),
            el_opt("MessageData", "RxMessageDataType"),
        ],
    )
    .element("MA_RxDataPayload", "MaWrapperType")
    .element("MA_TxDataPayloadCommand", "MaWrapperType");

    s
}
