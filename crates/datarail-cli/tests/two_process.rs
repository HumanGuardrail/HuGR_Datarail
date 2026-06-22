//! Genuine **two-process** transfer over a real TCP socket: `datarail send` (one OS process) ships a sealed
//! shipment to `datarail recv` (a *separate* OS process), which verifies → opens → offloads it to a sink file.
//! This exercises the cross-host product path across real process boundaries — not threads inside one test.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

/// Path to the built `datarail` binary (cargo sets this for integration tests of the bin's crate).
const BIN: &str = env!("CARGO_BIN_EXE_datarail");

/// Write a self-contained `rail.toml` to `dir` (no CWD dependency), returning its path.
fn write_spec(dir: &std::path::Path) -> std::path::PathBuf {
    const K: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    let spec = format!(
        "[route]\n\
         route_id = \"0x01010101010101010101010101010101\"\n\
         stream_id = \"0x02020202020202020202020202020202\"\n\
         aead = \"gcm-siv-256\"\n\
         guarantee = \"exactly-once\"\n\
         [onboarding]\n\
         max_record_len = 1024\n\
         required_prefix = \"evt:\"\n\
         [offloading]\n\
         max_record_len = 1024\n\
         required_prefix = \"evt:\"\n\
         [keys]\n\
         source_seed = \"{K}\"\n\
         dest_seed = \"{K}\"\n\
         dest_x25519_secret = \"{K}\"\n\
         tenant_secret = \"{K}\"\n"
    );
    let path = dir.join("rail.toml");
    std::fs::write(&path, spec).expect("write spec");
    path
}

#[test]
fn two_process_tcp_transfer_delivers_to_a_separate_recv() {
    let dir = std::env::temp_dir().join(format!("dr-2proc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let spec = write_spec(&dir);
    let sink = dir.join("out.txt");

    // Destination process: bind an OS-assigned port, announce it, offload one shipment to the sink file.
    let mut recv = Command::new(BIN)
        .arg("recv")
        .arg(&spec)
        .arg("--listen")
        .arg("127.0.0.1:0")
        .arg("--sink-file")
        .arg(&sink)
        .arg("--count")
        .arg("1")
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn recv");

    // Discover the bound address from recv's announced line.
    let stdout = recv.stdout.take().expect("recv stdout");
    let mut reader = BufReader::new(stdout);
    let addr = loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read recv stdout");
        assert!(n > 0, "recv closed stdout before announcing its address");
        if let Some(rest) = line.strip_prefix("DATARAIL-LISTENING ") {
            break rest.trim().to_owned();
        }
    };
    assert!(!addr.is_empty(), "no bound address announced");

    // Source process: a SEPARATE binary invocation connects and ships two records in one sealed cofre.
    let send = Command::new(BIN)
        .arg("send")
        .arg(&spec)
        .arg("--connect")
        .arg(&addr)
        .arg("evt:cross-process-1")
        .arg("evt:cross-process-2")
        .output()
        .expect("run send");
    assert!(
        send.status.success(),
        "send failed: {}",
        String::from_utf8_lossy(&send.stderr)
    );

    // The destination process finishes after one cofre.
    let status = recv.wait().expect("wait recv");
    assert!(status.success(), "recv exited non-zero");

    // Both records landed at the destination's sink, intact, across two real OS processes.
    let out = std::fs::read_to_string(&sink).expect("read sink");
    assert!(out.contains("evt:cross-process-1"), "sink missing record 1: {out:?}");
    assert!(out.contains("evt:cross-process-2"), "sink missing record 2: {out:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
