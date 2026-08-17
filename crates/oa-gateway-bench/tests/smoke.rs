//! The binary starts and an engine run prints `received`.

use std::process::Command;

#[test]
fn engine_smoke_prints_received() {
    let exe = env!("CARGO_BIN_EXE_oa-gateway-bench");
    let out = Command::new(exe)
        .args(["engine", "--duration", "200ms", "--warmup", "0"])
        .output()
        .expect("run oa-gateway-bench");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "status {:?} stdout={stdout} stderr={stderr}",
        out.status
    );
    assert!(
        stdout.contains("received"),
        "expected received in stdout, got {stdout}"
    );
}

#[test]
fn ping_smoke_prints_received() {
    let exe = env!("CARGO_BIN_EXE_oa-gateway-bench");
    let out = Command::new(exe)
        .args(["ping"])
        .output()
        .expect("run oa-gateway-bench ping");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "status {:?} stdout={stdout} stderr={stderr}",
        out.status
    );
    assert!(
        stdout.contains(r#"{"Ping":{"n":1}}"#),
        "expected Ping payload in stdout, got {stdout}"
    );
}
