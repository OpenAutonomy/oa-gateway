//! Live ActiveMQ Classic XML round-trip. Ignored unless a broker is running.
//!
//!   ./scripts/live-activemq.sh

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use oa_gateway_core::{ContentType, Engine, Envelope, RouteKey};
use oa_gateway_loopback::Loopback;
use oa_gateway_testing::fixtures::POSITION_REPORT_XML;
use oa_gateway_testing::stomp::{start_stomp_adapter, StompPeer};
use oa_gateway_testing::util::{unique_token, xml_marked};
use tokio::net::TcpStream;
use tokio::time::timeout;

fn broker_addr() -> SocketAddr {
    std::env::var("OAG_ACTIVEMQ_STOMP")
        .unwrap_or_else(|_| "127.0.0.1:61613".into())
        .parse()
        .expect("OAG_ACTIVEMQ_STOMP must be host:port")
}

async fn require_broker(addr: SocketAddr) {
    match timeout(Duration::from_secs(2), TcpStream::connect(addr)).await {
        Ok(Ok(_)) => {}
        _ => panic!(
            "no STOMP broker at {addr}; start compose/activemq.yml or run scripts/live-activemq.sh"
        ),
    }
}

#[tokio::test]
#[ignore = "requires ActiveMQ Classic STOMP (compose/activemq.yml)"]
async fn broker_xml_reaches_loopback() {
    let broker = broker_addr();
    require_broker(broker).await;

    let token = unique_token("in");
    let xml = xml_marked(POSITION_REPORT_XML, &token);

    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-b");
    let mut rx = loopback
        .subscribe(RouteKey::typed("PositionReport", "PositionReport"))
        .await
        .unwrap();

    let shutdown = start_stomp_adapter(
        engine,
        "stomp-live",
        broker,
        vec!["PositionReport".into()],
        Duration::from_millis(400),
    )
    .await;

    let mut peer = StompPeer::connect(broker).await;
    peer.send(
        "/topic/PositionReport",
        xml.as_bytes(),
        &[("content-type", "application/xml")],
    )
    .await;

    let got = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timeout waiting for loopback")
        .expect("closed");
    assert_eq!(got.route.topic, "PositionReport");
    assert_eq!(got.route.type_hint.as_deref(), Some("PositionReport"));
    assert_eq!(got.content_type, ContentType::xml());
    let body = std::str::from_utf8(&got.payload).unwrap();
    assert!(body.contains(&token), "missing marker {token}: {body}");

    shutdown.cancel();
}

#[tokio::test]
#[ignore = "requires ActiveMQ Classic STOMP (compose/activemq.yml)"]
async fn loopback_xml_reaches_broker() {
    let broker = broker_addr();
    require_broker(broker).await;

    let token = unique_token("out");
    let xml = xml_marked(POSITION_REPORT_XML, &token);

    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-a");
    let shutdown = start_stomp_adapter(
        engine,
        "stomp-live",
        broker,
        vec!["PositionReport".into()],
        Duration::from_millis(400),
    )
    .await;

    let mut peer = StompPeer::connect(broker).await;
    peer.subscribe("live-out", "/topic/PositionReport").await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    loopback
        .publish(
            Envelope::new(
                RouteKey::typed("PositionReport", "PositionReport"),
                Bytes::from(xml.clone()),
            )
            .with_content_type(ContentType::xml()),
        )
        .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let msg = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let frame = timeout(remaining, peer.recv())
            .await
            .expect("timeout waiting for STOMP MESSAGE");
        assert_eq!(frame.command, "MESSAGE");
        let body = String::from_utf8_lossy(&frame.body);
        if body.contains(&token) {
            break frame;
        }
    };
    assert_eq!(msg.header("destination"), Some("/topic/PositionReport"));
    if let Some(ct) = msg.header("content-type") {
        let ct = ct.to_ascii_lowercase();
        assert!(
            ct.contains("xml") || ct.contains("octet-stream") || ct.contains("text/plain"),
            "unexpected content-type {ct}"
        );
    }

    shutdown.cancel();
}

#[tokio::test]
#[ignore = "requires ActiveMQ Classic STOMP (compose/activemq.yml)"]
async fn inbound_does_not_echo_duplicate() {
    let broker = broker_addr();
    require_broker(broker).await;

    let token = unique_token("echo");
    let xml = xml_marked(POSITION_REPORT_XML, &token);

    let engine = Arc::new(Engine::new());
    let shutdown = start_stomp_adapter(
        engine,
        "stomp-live",
        broker,
        vec!["PositionReport".into()],
        Duration::from_millis(400),
    )
    .await;

    let mut watcher = StompPeer::connect(broker).await;
    watcher
        .subscribe("live-watch", "/topic/PositionReport")
        .await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    let mut sender = StompPeer::connect(broker).await;
    sender
        .send(
            "/topic/PositionReport",
            xml.as_bytes(),
            &[("content-type", "application/xml")],
        )
        .await;

    loop {
        let frame = timeout(Duration::from_secs(5), watcher.recv())
            .await
            .expect("timeout waiting for original fan-out");
        let body = String::from_utf8_lossy(&frame.body);
        if body.contains(&token) {
            break;
        }
    }

    match timeout(Duration::from_millis(500), watcher.recv()).await {
        Err(_) => {}
        Ok(dup) => {
            let body = String::from_utf8_lossy(&dup.body);
            if body.contains(&token) {
                panic!("oa-gateway echoed inbound XML back onto the broker: {body}");
            }
        }
    }

    shutdown.cancel();
}
