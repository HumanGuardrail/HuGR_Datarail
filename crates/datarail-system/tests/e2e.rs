//! Capstone: the system comes alive. These tests run the REAL crates together — TCP broker + durable retained
//! log + durable offsets + erasure replication + a remote (TCP) blob store — and prove the composed behaviour.

use std::path::PathBuf;

use datarail_broker::{serve, Client, Server};
use datarail_offsets::{FileOffsets, OffsetStore};
use datarail_topic::Topic;

fn tmp(tag: &str) -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("datarail-system-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// THE capstone: a durable, networked broker survives a "restart" (a fresh broker over the same on-disk state)
/// and a consumer group RESUMES from its durably-committed offset — over a real TCP socket, end to end.
#[test]
fn durable_networked_broker_resumes_from_committed_offset_after_restart() {
    const N: u64 = 1000;
    const CONSUMED: usize = 600;
    let dt = tmp("topic");
    let dof = tmp("offsets");
    let mut offsets = Vec::new();

    // ---- Phase 1: a broker serves a durable topic + durable offsets over TCP; a client produces + consumes. ----
    {
        let topic = Topic::open(&dt, 1 << 16).expect("topic");
        let off = FileOffsets::open(&dof).expect("offsets");
        let addr = serve(Server::new(topic, off), "127.0.0.1:0").expect("serve1");
        let mut c = Client::connect(addr).expect("connect1");
        for i in 0..N {
            let o = c
                .produce(format!("k{}", i % 50).as_bytes(), format!("v{i}").as_bytes())
                .expect("produce");
            offsets.push(o);
        }
        // Consume the first CONSUMED records, then durably commit the NEXT offset as the resume point.
        for _ in 0..CONSUMED {
            c.poll("g").expect("poll").expect("a record");
        }
        c.commit("g", offsets[CONSUMED]).expect("commit");
        // client drops here; the phase-1 serve thread lingers (daemon) but receives no more commands.
    }

    // ---- Phase 2: "restart" — a brand-new broker over the SAME on-disk dirs. Durable state must have survived. ----
    let topic2 = Topic::open(&dt, 1 << 16).expect("reopen topic");
    let off2 = FileOffsets::open(&dof).expect("reopen offsets");
    assert_eq!(off2.fetch("g"), Some(offsets[CONSUMED]), "committed offset did NOT survive the restart");

    let addr2 = serve(Server::new(topic2, off2), "127.0.0.1:0").expect("serve2");
    let mut c2 = Client::connect(addr2).expect("connect2");
    let mut got = Vec::new();
    while let Some(rec) = c2.poll("g").expect("poll2") {
        got.push(rec);
    }

    // The group resumed EXACTLY at the committed offset and replayed only the un-consumed tail — nothing lost,
    // nothing re-delivered — over the network, after a restart.
    assert_eq!(got.len(), usize::try_from(N).expect("fits") - CONSUMED, "did not resume the exact un-consumed tail");
    assert_eq!(got[0].0, offsets[CONSUMED], "did not resume at the committed offset");
    for (j, rec) in got.iter().enumerate() {
        let i = CONSUMED + j;
        assert_eq!(rec.2, format!("v{i}").into_bytes(), "payload corrupted/lost across restart at record {i}");
    }
    let _ = std::fs::remove_dir_all(&dt);
    let _ = std::fs::remove_dir_all(&dof);
}

/// Node-loss durability, end to end over the network: erasure shards live on a REMOTE (TCP) blob store; lose any
/// `m` of them and the record still reconstructs over the wire — 1.5x storage, 2-failure tolerance, remote tier.
#[test]
fn record_survives_losing_m_shards_on_a_remote_blob_store() {
    use datarail_blobstore::{BlobStore, MemBlob};
    use datarail_netblob::{serve as serve_blob, NetBlob};
    use datarail_replication::ErasureStore;

    let addr = serve_blob(MemBlob::new()).expect("serve blob");
    let mut store = ErasureStore::new(NetBlob::new(addr), 4, 2).expect("erasure store"); // k=4, m=2 → survives 2 losses
    let data: Vec<u8> = b"datarail-replicated-record-payload-".iter().copied().cycle().take(4096).collect();
    store.put("rec1", &data).expect("put shards to the remote store");

    // A second client destroys m=2 of the 6 shards on the REMOTE store (simulating two node/disk losses).
    let mut killer = NetBlob::new(addr);
    assert!(killer.delete("rec1/0").expect("delete shard 0"));
    assert!(killer.delete("rec1/3").expect("delete shard 3"));

    // Reconstruct over the network from the 4 surviving shards.
    let got = store.get("rec1").expect("get").expect("must reconstruct after losing m shards");
    assert_eq!(got, data, "remote erasure failed to reconstruct after losing m shards");
}
