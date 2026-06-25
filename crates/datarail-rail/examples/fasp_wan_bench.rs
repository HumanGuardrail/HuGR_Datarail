//! S3 goodput bench (FASP-UDP-TRANSPORT.md): move the SAME payload A→B over `FaspLink` (UDP, our delay-based
//! controller) and over kernel TCP, on the same loopback. Run it under `tc netem` (applied externally on `lo`)
//! so a real kernel injects loss + delay equally on both — turning the sim's 16–42× into a real-socket number,
//! whatever it is. Prints `fasp_mbps=… tcp_mbps=… ratio=…` for the harness to parse.
//!
//! Usage: `cargo run --release --example fasp_wan_bench -p datarail-rail -- [MiB]`

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use datarail_rail::reliable_udp::{FaspCfg, FaspLink, MAX_FASP_PAYLOAD};

fn loopback() -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
}

/// Goodput in MB/s (decimal megabytes), converting the byte count losslessly via `u32` (the MB count is tiny).
fn mbps(bytes: u64, secs: f64) -> f64 {
    let mb = f64::from(u32::try_from(bytes / 1_000_000).unwrap_or(u32::MAX));
    if secs > 0.0 {
        mb / secs
    } else {
        0.0
    }
}

/// Move `chunks` × [`MAX_FASP_PAYLOAD`] bytes A→B over a real UDP `FaspLink`; return MB/s of steady-state transfer.
fn bench_fasp(chunks: u64) -> f64 {
    // Generous window so FASP can fill a high-RTT pipe; under loss it HOLDS (Karn) while TCP would collapse.
    let cfg = FaspCfg {
        max_inflight: 8192,
        max_window: 8192.0,
        init_window: 64.0,
        min_window: 8.0,
        rto: Duration::from_millis(120),
        queue_threshold: Duration::from_millis(10),
        loss_sim_drop_every: 0, // real loss comes from tc netem, not the simulator
    };
    let mut a = FaspLink::bind(loopback(), cfg).expect("bind a");
    let mut b = FaspLink::bind(loopback(), cfg).expect("bind b");
    let a_addr = a.local_addr().expect("a addr");
    let b_addr = b.local_addr().expect("b addr");
    a.set_peer(b_addr);
    b.set_peer(a_addr);

    let start = Instant::now();
    let rx = thread::spawn(move || {
        let mut got = 0u64;
        while got < chunks {
            while b.recv().expect("recv").is_some() {
                got += 1;
            }
        }
    });
    let payload = [0u8; MAX_FASP_PAYLOAD];
    for _ in 0..chunks {
        a.send(&payload).expect("send");
    }
    while a.inflight_len() > 0 {
        a.pump().expect("pump");
    }
    rx.join().expect("rx join");
    mbps(chunks * MAX_FASP_PAYLOAD as u64, start.elapsed().as_secs_f64())
}

/// Move `total` bytes A→B over kernel TCP on the same loopback; return MB/s of steady-state transfer (timing
/// starts after `connect`, so the comparison is goodput, not the TCP handshake).
fn bench_tcp(total: u64) -> f64 {
    let listener = TcpListener::bind(loopback()).expect("tcp bind");
    let addr = listener.local_addr().expect("tcp addr");
    let acc = thread::spawn(move || {
        let (mut s, _) = listener.accept().expect("accept");
        let mut sink = vec![0u8; 1 << 16];
        let mut got = 0u64;
        while got < total {
            let n = s.read(&mut sink).expect("read");
            if n == 0 {
                break;
            }
            got += n as u64;
        }
    });
    let mut c = TcpStream::connect(addr).expect("connect");
    let start = Instant::now();
    let buf = vec![0u8; 1 << 16];
    let mut sent = 0u64;
    while sent < total {
        let remaining = total - sent;
        let take = remaining.min(buf.len() as u64);
        let end = usize::try_from(take).unwrap_or(buf.len());
        let n = c.write(&buf[..end]).expect("write");
        sent += n as u64;
    }
    c.flush().expect("flush");
    acc.join().expect("acc join");
    mbps(total, start.elapsed().as_secs_f64())
}

fn main() {
    let mib: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(32);
    let total = mib * 1024 * 1024;
    let chunks = total / MAX_FASP_PAYLOAD as u64;
    let fasp = bench_fasp(chunks);
    let tcp = bench_tcp(chunks * MAX_FASP_PAYLOAD as u64);
    let ratio = if tcp > 0.0 { fasp / tcp } else { 0.0 };
    println!("fasp_mbps={fasp:.2} tcp_mbps={tcp:.2} ratio={ratio:.2}");
}
