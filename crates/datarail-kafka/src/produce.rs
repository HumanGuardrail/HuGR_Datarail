//! The `Produce` (API 0) request: parse the request envelope + the v2 `RecordBatch` blob, extract each record's
//! value bytes (the payload a datarail terminal will seal), and build the response. Uncompressed batches only
//! (a producer with compression disabled — the default for the common path); a compressed batch is a clear error.

use std::io;

use crate::codec::{write_response_header, Reader, Writer};

/// A produced partition: its index and the record VALUES extracted from the batch.
#[derive(Debug, Clone)]
pub struct ProducedPartition {
    /// Partition index.
    pub partition: i32,
    /// Record value payloads, in order.
    pub values: Vec<Vec<u8>>,
}

/// A produced topic: its name and the partitions in this request.
#[derive(Debug, Clone)]
pub struct ProducedTopic {
    /// Topic name.
    pub name: String,
    /// Partitions produced to.
    pub partitions: Vec<ProducedPartition>,
}

/// Take a varint-length-prefixed byte run from `reader` (`-1` ⇒ `None`/absent), returning the bytes.
fn take_varint_bytes(reader: &mut Reader) -> io::Result<Option<Vec<u8>>> {
    let len = reader.varint()?;
    if len < 0 {
        return Ok(None);
    }
    let n = usize::try_from(len).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad varint length"))?;
    Ok(Some(reader.take(n)?.to_vec()))
}

/// Parse a producer's records blob into the record VALUE payloads. Handles BOTH the v2 `RecordBatch` (magic 2,
/// modern clients) and the legacy `MessageSet` (magic 0/1, older clients and some librdkafka fallbacks) — they
/// share a layout up to the magic byte at offset 16 (`int64`, `int32`, 4 bytes, then magic), so we read that far
/// and dispatch. Uncompressed only — a compressed entry is a clear error.
///
/// # Errors
/// [`io::Error`] on a malformed blob, an unsupported magic byte, or a compressed entry.
pub fn parse_record_batch(blob: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let mut reader = Reader::new(blob);
    let mut values = Vec::new();
    while !reader.is_empty() {
        let _offset = reader.int64()?; // baseOffset (v2) / offset (legacy)
        let _length = reader.int32()?; // batchLength (v2) / messageSize (legacy)
        let _crc_or_epoch = reader.uint32()?; // partitionLeaderEpoch (v2) / crc (legacy)
        let magic = reader.int8()?;
        match magic {
            2 => parse_v2_records(&mut reader, &mut values)?,
            0 | 1 => parse_legacy_message(&mut reader, magic, &mut values)?,
            other => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unsupported record magic {other}")));
            }
        }
    }
    Ok(values)
}

/// Parse the v2 `RecordBatch` fields after the magic byte, pushing each record's value.
fn parse_v2_records(reader: &mut Reader, values: &mut Vec<Vec<u8>>) -> io::Result<()> {
    let _crc = reader.uint32()?;
    let attributes = reader.int16()?;
    if attributes & 0x07 != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "compressed batches not supported"));
    }
    let _last_offset_delta = reader.int32()?;
    let _base_timestamp = reader.int64()?;
    let _max_timestamp = reader.int64()?;
    let _producer_id = reader.int64()?;
    let _producer_epoch = reader.int16()?;
    let _base_sequence = reader.int32()?;
    let record_count = reader.int32()?;
    let n = usize::try_from(record_count)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad record count"))?;
    for _ in 0..n {
        let _length = reader.varint()?;
        let _attributes = reader.int8()?;
        let _timestamp_delta = reader.varlong()?;
        let _offset_delta = reader.varint()?;
        let _key = take_varint_bytes(reader)?;
        if let Some(value) = take_varint_bytes(reader)? {
            values.push(value);
        }
        let header_count = reader.varint()?;
        let hc = usize::try_from(header_count.max(0)).unwrap_or(0);
        for _ in 0..hc {
            let _hk = take_varint_bytes(reader)?;
            let _hv = take_varint_bytes(reader)?;
        }
    }
    Ok(())
}

/// Parse one legacy `MessageSet` message (magic 0/1) after the magic byte, pushing its value. The crc and the
/// preceding `int64` offset + `int32` size were already consumed by the caller; key/value are classic `INT32`-
/// length `BYTES`.
fn parse_legacy_message(reader: &mut Reader, magic: i8, values: &mut Vec<Vec<u8>>) -> io::Result<()> {
    let attributes = reader.int8()?;
    if attributes & 0x07 != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "compressed messages not supported"));
    }
    if magic >= 1 {
        let _timestamp = reader.int64()?;
    }
    let _key = reader.nullable_bytes()?;
    if let Some(value) = reader.nullable_bytes()? {
        values.push(value);
    }
    Ok(())
}

/// Parse a `Produce` request body (after the request header) at `version`, returning the produced topics.
///
/// # Errors
/// [`io::Error`] if the request or any embedded record batch is malformed.
pub fn parse_produce(reader: &mut Reader, version: i16) -> io::Result<Vec<ProducedTopic>> {
    if version >= 3 {
        let _transactional_id = reader.nullable_string()?;
    }
    let _acks = reader.int16()?;
    let _timeout_ms = reader.int32()?;
    let topic_count = reader.int32()?;
    // Bound the count by the bytes actually remaining (each element is ≥1 byte): a small frame cannot claim
    // billions of topics/partitions and drive an over-allocation (audit K3). A genuine over-claim then fails
    // fast when the per-element reads run out of buffer.
    let tc = usize::try_from(topic_count.max(0)).unwrap_or(0).min(reader.remaining().len());
    let mut topics = Vec::with_capacity(tc);
    for _ in 0..tc {
        let name = reader.string()?;
        let part_count = reader.int32()?;
        let pc = usize::try_from(part_count.max(0)).unwrap_or(0).min(reader.remaining().len());
        let mut partitions = Vec::with_capacity(pc);
        for _ in 0..pc {
            let partition = reader.int32()?;
            let values = match reader.nullable_bytes()? {
                Some(blob) => parse_record_batch(&blob)?,
                None => Vec::new(),
            };
            partitions.push(ProducedPartition { partition, values });
        }
        topics.push(ProducedTopic { name, partitions });
    }
    Ok(topics)
}

/// Build the full `Produce` response (header v0 + body) at `version`. `base_offset_for` yields the base offset
/// assigned to each `(topic, partition)` — the offset of that batch's first record.
#[must_use]
pub fn produce_response(
    version: i16,
    correlation_id: i32,
    topics: &[ProducedTopic],
    base_offset_for: &mut dyn FnMut(&str, i32, usize) -> i64,
) -> Vec<u8> {
    let mut w = Writer::new();
    write_response_header(&mut w, correlation_id, false); // Produce response header is v0 for versions ≤ 8

    let count = i32::try_from(topics.len()).unwrap_or(0);
    w.int32(count);
    for topic in topics {
        w.string(&topic.name);
        let pcount = i32::try_from(topic.partitions.len()).unwrap_or(0);
        w.int32(pcount);
        for part in &topic.partitions {
            let base = base_offset_for(&topic.name, part.partition, part.values.len());
            w.int32(part.partition);
            w.int16(0); // error_code NONE
            w.int64(base); // base_offset
            if version >= 2 {
                w.int64(-1); // log_append_time = -1 (CreateTime)
            }
            if version >= 5 {
                w.int64(0); // log_start_offset
            }
        }
    }
    if version >= 1 {
        w.int32(0); // throttle_time_ms
    }
    w.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::{parse_produce, parse_record_batch};
    use crate::codec::Writer;

    /// Build a minimal uncompressed v2 `RecordBatch` carrying the given values (null keys, no headers).
    fn record_batch(values: &[&[u8]]) -> Vec<u8> {
        let mut recs = Writer::new();
        for (i, v) in values.iter().enumerate() {
            let mut r = Writer::new();
            r.int8(0); // attributes
            r.varlong(0); // timestamp delta
            r.varint(i32::try_from(i).unwrap()); // offset delta
            r.varint(-1); // null key
            r.varint(i32::try_from(v.len()).unwrap());
            r.raw(v);
            r.varint(0); // header count
            let body = r.into_bytes();
            recs.varint(i32::try_from(body.len()).unwrap());
            recs.raw(&body);
        }
        let records = recs.into_bytes();

        let mut b = Writer::new();
        b.int32(0); // partition_leader_epoch
        b.int8(2); // magic
        b.uint32(0); // crc (not validated on parse)
        b.int16(0); // attributes (uncompressed)
        b.int32(i32::try_from(values.len().saturating_sub(1)).unwrap()); // last_offset_delta
        b.int64(0); // base ts
        b.int64(0); // max ts
        b.int64(-1); // producer id
        b.int16(-1); // producer epoch
        b.int32(-1); // base sequence
        b.int32(i32::try_from(values.len()).unwrap()); // record count
        b.raw(&records);
        let after = b.into_bytes();

        let mut full = Writer::new();
        full.int64(0); // base offset
        full.int32(i32::try_from(after.len()).unwrap()); // batch length
        full.raw(&after);
        full.into_bytes()
    }

    /// Build a legacy v0 `MessageSet` (magic 0) carrying one null-key value — the format librdkafka sent live.
    fn legacy_messageset(value: &[u8]) -> Vec<u8> {
        let mut msg = Writer::new();
        msg.uint32(0); // crc (not validated)
        msg.int8(0); // magic 0
        msg.int8(0); // attributes (uncompressed)
        msg.int32(-1); // null key
        msg.int32(i32::try_from(value.len()).unwrap());
        msg.raw(value);
        let body = msg.into_bytes();
        let mut set = Writer::new();
        set.int64(0); // offset
        set.int32(i32::try_from(body.len()).unwrap()); // message size
        set.raw(&body);
        set.into_bytes()
    }

    #[test]
    fn over_claimed_count_does_not_over_allocate() {
        // topic_count = i32::MAX with no topic bytes: the remaining-bytes bound keeps allocation tiny (audit K3).
        // Before the bound, `Vec::with_capacity(i32::MAX as usize)` aborts the process.
        let mut req = Writer::new();
        req.nullable_string(None);
        req.int16(1);
        req.int32(0);
        req.int32(i32::MAX);
        let bytes = req.into_bytes();
        let mut reader = crate::codec::Reader::new(&bytes);
        let topics = parse_produce(&mut reader, 7).expect("bounded parse, no over-alloc");
        assert!(topics.len() < 1024, "count must be bounded by remaining bytes, got {}", topics.len());
    }

    #[test]
    fn legacy_messageset_value_extracted() {
        let blob = legacy_messageset(b"evt:legacy");
        let values = parse_record_batch(&blob).expect("parse legacy");
        assert_eq!(values, vec![b"evt:legacy".to_vec()]);
    }

    #[test]
    fn record_batch_values_extracted() {
        let blob = record_batch(&[b"evt:one", b"evt:two", b"evt:three"]);
        let values = parse_record_batch(&blob).expect("parse");
        assert_eq!(values, vec![b"evt:one".to_vec(), b"evt:two".to_vec(), b"evt:three".to_vec()]);
    }

    #[test]
    fn produce_v7_request_round_trips_records() {
        let batch = record_batch(&[b"evt:a", b"evt:b"]);
        let mut req = Writer::new();
        req.nullable_string(None); // transactional_id (v3+)
        req.int16(1); // acks
        req.int32(1000); // timeout
        req.int32(1); // 1 topic
        req.string("events");
        req.int32(1); // 1 partition
        req.int32(0); // partition 0
        req.bytes(&batch); // records as NULLABLE_BYTES (int32 len + bytes)
        let bytes = req.into_bytes();

        let mut reader = crate::codec::Reader::new(&bytes);
        let topics = parse_produce(&mut reader, 7).expect("parse produce");
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].name, "events");
        assert_eq!(topics[0].partitions[0].values, vec![b"evt:a".to_vec(), b"evt:b".to_vec()]);
    }

    #[test]
    fn compressed_batch_is_rejected() {
        let mut b = Writer::new();
        b.int64(0);
        b.int32(20);
        b.int32(0);
        b.int8(2); // magic
        b.uint32(0);
        b.int16(1); // attributes: gzip compression bit set
        b.int32(0);
        b.int64(0);
        b.int64(0);
        b.int64(-1);
        b.int16(-1);
        b.int32(-1);
        b.int32(1);
        assert!(parse_record_batch(&b.into_bytes()).is_err());
    }
}
