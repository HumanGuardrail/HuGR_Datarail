//! A zero-dependency `PostgreSQL` sink: a hand-rolled implementation of the frontend/backend wire
//! protocol version 3 over a plain [`std::net::TcpStream`]. Each delivered record lands as **one row**
//! in a target table's single column via the `COPY ... FROM STDIN` sub-protocol (text format).
//!
//! Scope and honesty:
//! - Auth supported: trust ([`AuthenticationOk`]), cleartext password, and `MD5` password (the `MD5`
//!   digest is hand-rolled here, RFC 1321). `SCRAM` (`SASL`) authentication is **not** supported and
//!   yields a clear [`io::ErrorKind::Unsupported`] error.
//! - No `TLS`: the connection is plaintext. A real deployment must tunnel this over a secure transport.
//! - Records are raw bytes landing into a **text** column. Bytes are escaped per the `COPY` text format
//!   (backslash, tab, newline, carriage-return); no further encoding is applied.
//!
//! [`AuthenticationOk`]: https://www.postgresql.org/docs/current/protocol-message-formats.html

use std::io::{self, Read, Write};
use std::net::TcpStream;

/// Protocol version 3.0 magic, sent in the startup message (`0x0003_0000`).
const PROTOCOL_V3: i32 = 196_608;

/// Hard cap on a single backend message body we are willing to allocate (defensive: the length is
/// attacker-influenced). 64 MiB is far beyond any control message this client expects.
const MAX_BACKEND_MSG: usize = 64 * 1024 * 1024;

/// Connection + landing configuration for a [`PostgresSink`].
///
/// `table` and `column` are operator-supplied identifiers (not untrusted record data); they are still
/// validated to reject a double-quote or NUL so they cannot break the generated `COPY` command.
#[derive(Debug, Clone)]
pub struct PgConfig {
    /// Server host (name or address).
    pub host: String,
    /// Server port (`PostgreSQL` default is 5432).
    pub port: u16,
    /// Role to authenticate as.
    pub user: String,
    /// Optional password (required only if the server requests cleartext or `MD5` auth).
    pub password: Option<String>,
    /// Target database name.
    pub dbname: String,
    /// Target table identifier.
    pub table: String,
    /// Target column identifier (each record lands as one value here).
    pub column: String,
}

impl PgConfig {
    /// A configuration with the default port (5432) and no password. Set [`PgConfig::password`] and
    /// [`PgConfig::port`] afterwards if needed.
    #[must_use]
    pub fn new(host: String, user: String, dbname: String, table: String, column: String) -> Self {
        Self {
            host,
            port: 5432,
            user,
            password: None,
            dbname,
            table,
            column,
        }
    }
}

/// A `PostgreSQL` sink speaking protocol v3 over a plaintext TCP connection. Construct with
/// [`PostgresSink::connect`]; commit batches via the [`crate::Sink`] trait.
#[derive(Debug)]
pub struct PostgresSink {
    stream: TcpStream,
    table: String,
    column: String,
}

impl PostgresSink {
    /// Connect, authenticate, and ready the sink for `COPY` batches.
    ///
    /// Performs the startup handshake, handles the authentication request (trust / cleartext / `MD5`),
    /// and drains until the server is `ReadyForQuery`.
    ///
    /// # Errors
    /// Returns [`io::Error`] if the TCP connection fails, an identifier is invalid, the backend reports
    /// an error, a malformed message is received, or the requested auth method is unsupported (`SCRAM`).
    pub fn connect(cfg: PgConfig) -> io::Result<Self> {
        validate_ident(&cfg.table)?;
        validate_ident(&cfg.column)?;

        let mut stream = TcpStream::connect((cfg.host.as_str(), cfg.port))?;
        let startup = startup_bytes(&cfg.user, &cfg.dbname)?;
        stream.write_all(&startup)?;
        authenticate(&mut stream, &cfg)?;

        Ok(Self {
            stream,
            table: cfg.table,
            column: cfg.column,
        })
    }

    /// Land `records` via `COPY <table>(<column>) FROM STDIN` (text format). Caller guarantees non-empty + NUL-free.
    fn copy_in(&mut self, records: &[Vec<u8>]) -> io::Result<()> {
        let mut query = format!("COPY \"{}\" (\"{}\") FROM STDIN", self.table, self.column).into_bytes();
        query.push(0); // simple Query is a NUL-terminated C string
        send(&mut self.stream, b'Q', &query)?;
        // Expect CopyInResponse ('G'); tolerate informational messages, fail on ErrorResponse.
        loop {
            let (tag, body) = read_msg(&mut self.stream)?;
            match tag {
                b'G' => break,
                b'E' => return Err(backend_error(&body)),
                b'Z' => return Err(invalid("server was not ready to accept COPY-IN data")),
                _ => {}
            }
        }
        for record in records {
            let mut frame = escape_copy(record);
            frame.push(b'\n'); // row terminator
            send(&mut self.stream, b'd', &frame)?;
        }
        send(&mut self.stream, b'c', &[])?; // CopyDone
        // Expect CommandComplete ('C') then ReadyForQuery ('Z').
        loop {
            let (tag, body) = read_msg(&mut self.stream)?;
            match tag {
                b'Z' => break,
                b'E' => return Err(backend_error(&body)),
                _ => {} // CommandComplete ('C') and informational messages
            }
        }
        Ok(())
    }

    /// The transaction body for `commit_at` (between `BEGIN` and `COMMIT`/`ROLLBACK`): read the current watermark
    /// under a row lock; if this batch already landed, no-op; otherwise land the records + advance the watermark.
    fn txn_body(&mut self, stream_hex: &str, watermark: i64, records: &[Vec<u8>]) -> io::Result<()> {
        let current = read_watermark(&mut self.stream, stream_hex, true)?; // FOR UPDATE
        if current >= watermark {
            return Ok(()); // already landed (idempotent replay) — the COMMIT makes it a clean no-op
        }
        if !records.is_empty() {
            self.copy_in(records)?;
        }
        let upsert = format!(
            "INSERT INTO datarail_watermark (stream, seq) VALUES ('\\x{stream_hex}', {watermark}) \
             ON CONFLICT (stream) DO UPDATE SET seq = EXCLUDED.seq"
        );
        query_simple(&mut self.stream, &upsert)?;
        Ok(())
    }
}

/// Create the watermark table if absent (idempotent). The dedup watermark lives in the sink's own DB.
fn ensure_watermark_table(stream: &mut (impl Read + Write)) -> io::Result<()> {
    query_simple(stream, "CREATE TABLE IF NOT EXISTS datarail_watermark (stream bytea PRIMARY KEY, seq bigint NOT NULL)")
        .map(|_| ())
}

/// Read the current watermark seq for `stream_hex` (`0` if no row), optionally with `FOR UPDATE` to lock it
/// inside a transaction. `stream_hex` is a hex string (no injection); the seq is an integer.
fn read_watermark(stream: &mut (impl Read + Write), stream_hex: &str, for_update: bool) -> io::Result<i64> {
    let lock = if for_update { " FOR UPDATE" } else { "" };
    let sql = format!("SELECT seq FROM datarail_watermark WHERE stream = '\\x{stream_hex}'{lock}");
    let row = query_simple(stream, &sql)?;
    let seq = row
        .and_then(|b| std::str::from_utf8(&b).ok().and_then(|s| s.trim().parse::<i64>().ok()))
        .unwrap_or(0);
    Ok(seq)
}

/// Run a simple `Query`, draining to `ReadyForQuery`. Returns the first field of the first `DataRow` (if any) —
/// used both for value-less statements (`BEGIN`/`COMMIT`/DDL/`INSERT`) and single-scalar `SELECT`s. Defensive
/// against short/garbage backend messages; an `ErrorResponse` becomes an `io::Error`.
fn query_simple(stream: &mut (impl Read + Write), sql: &str) -> io::Result<Option<Vec<u8>>> {
    let mut q = sql.as_bytes().to_vec();
    q.push(0); // NUL-terminated C string
    send(stream, b'Q', &q)?;
    let mut first: Option<Vec<u8>> = None;
    loop {
        let (tag, body) = read_msg(stream)?;
        match tag {
            b'D' if first.is_none() => {
                // DataRow: int16 field count, then per field [int32 len][bytes] (len -1 = NULL).
                let nfields = body.get(0..2).and_then(|b| <[u8; 2]>::try_from(b).ok()).map(i16::from_be_bytes);
                if nfields.is_some_and(|n| n >= 1) {
                    let len = body.get(2..6).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(i32::from_be_bytes);
                    first = match len {
                        Some(l) if l >= 0 => {
                            let n = usize::try_from(l).unwrap_or(0);
                            body.get(6..6 + n).map(<[u8]>::to_vec)
                        }
                        _ => Some(Vec::new()), // NULL field
                    };
                }
            }
            b'E' => return Err(backend_error(&body)),
            b'Z' => break, // ReadyForQuery
            _ => {} // RowDescription 'T', CommandComplete 'C', NoticeResponse, etc.
        }
    }
    Ok(first)
}

impl crate::Sink for PostgresSink {
    /// Land each record as one row in the configured `table(column)` using `COPY ... FROM STDIN`
    /// (text format). Records are raw bytes escaped per the `COPY` text format; they land into a text
    /// column. An empty batch is a no-op.
    fn commit(&mut self, records: &[Vec<u8>]) -> io::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        // The COPY text format cannot represent a NUL byte; reject up front with a clear error rather than
        // letting a single crafted record fail mid-COPY (which would also leave the connection mid-stream).
        if records.iter().any(|r| r.contains(&0)) {
            return Err(invalid("record contains a NUL byte, which the Postgres text COPY format cannot carry"));
        }
        self.copy_in(records)
    }
}

impl crate::TxnSink for PostgresSink {
    /// Tier-A exactly-once (see `EXACTLY-ONCE-DESIGN.md`): land `records` AND advance the watermark for `stream`
    /// in ONE Postgres transaction. Idempotent — a replayed batch (`watermark <= stored`) is a committed no-op.
    fn commit_at(&mut self, records: &[Vec<u8>], stream: &[u8], watermark: u64) -> io::Result<()> {
        if records.iter().any(|r| r.contains(&0)) {
            return Err(invalid("record contains a NUL byte, which the Postgres text COPY format cannot carry"));
        }
        let stream_hex = hex(stream);
        let wm = i64::try_from(watermark).unwrap_or(i64::MAX);
        ensure_watermark_table(&mut self.stream)?;
        query_simple(&mut self.stream, "BEGIN")?;
        // Run the transaction body; on ANY error roll back so the connection is reusable (not stuck in a failed txn).
        match self.txn_body(&stream_hex, wm, records) {
            Ok(()) => query_simple(&mut self.stream, "COMMIT").map(|_| ()),
            Err(e) => {
                let _ = query_simple(&mut self.stream, "ROLLBACK");
                Err(e)
            }
        }
    }

    fn resume_watermark(&mut self, stream: &[u8]) -> io::Result<u64> {
        ensure_watermark_table(&mut self.stream)?;
        let cur = read_watermark(&mut self.stream, &hex(stream), false)?;
        Ok(u64::try_from(cur).unwrap_or(0))
    }
}

// ---------------------------------------------------------------------------------------------------
// Message framing (pure, unit-testable)
// ---------------------------------------------------------------------------------------------------

/// Build the v3 `StartupMessage` bytes: Int32 length, Int32 protocol, then `user`/`database` parameters.
fn startup_bytes(user: &str, dbname: &str) -> io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&PROTOCOL_V3.to_be_bytes());
    payload.extend_from_slice(b"user\0");
    payload.extend_from_slice(user.as_bytes());
    payload.push(0);
    payload.extend_from_slice(b"database\0");
    payload.extend_from_slice(dbname.as_bytes());
    payload.push(0);
    payload.push(0); // end of parameter list

    let total = i32::try_from(payload.len() + 4).map_err(|_| too_large())?;
    let mut buf = Vec::with_capacity(payload.len() + 4);
    buf.extend_from_slice(&total.to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Build a tagged frontend message: 1-byte tag, Int32 length (covers length + body), then body.
fn frame_bytes(tag: u8, body: &[u8]) -> io::Result<Vec<u8>> {
    let total = i32::try_from(body.len() + 4).map_err(|_| too_large())?;
    let mut buf = Vec::with_capacity(body.len() + 5);
    buf.push(tag);
    buf.extend_from_slice(&total.to_be_bytes());
    buf.extend_from_slice(body);
    Ok(buf)
}

/// Write one tagged frontend message to the stream.
fn send(stream: &mut impl Write, tag: u8, body: &[u8]) -> io::Result<()> {
    stream.write_all(&frame_bytes(tag, body)?)
}

/// Read one backend message: 1-byte tag, Int32 length, then `length - 4` body bytes. Defensive against
/// short/garbage headers and absurd lengths.
fn read_msg(stream: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; 5];
    stream.read_exact(&mut header)?;
    let tag = header[0];
    let len = i32::from_be_bytes([header[1], header[2], header[3], header[4]]);
    if len < 4 {
        return Err(invalid("backend message length below minimum"));
    }
    let body_len = usize::try_from(len - 4).map_err(|_| invalid("negative backend body length"))?;
    if body_len > MAX_BACKEND_MSG {
        return Err(invalid("backend message exceeds maximum allowed size"));
    }
    let mut body = vec![0u8; body_len];
    stream.read_exact(&mut body)?;
    Ok((tag, body))
}

/// Read a big-endian `i32` at `offset` from a backend body, defensively.
fn read_be_i32(body: &[u8], offset: usize) -> io::Result<i32> {
    let slice = body
        .get(offset..offset + 4)
        .ok_or_else(|| invalid("backend message too short for a 32-bit field"))?;
    let arr: [u8; 4] = slice.try_into().unwrap_or([0; 4]);
    Ok(i32::from_be_bytes(arr))
}

// ---------------------------------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------------------------------

/// Drive the authentication exchange until `ReadyForQuery` ('Z').
fn authenticate(stream: &mut (impl Read + Write), cfg: &PgConfig) -> io::Result<()> {
    loop {
        let (tag, body) = read_msg(stream)?;
        match tag {
            b'R' => {
                let code = read_be_i32(&body, 0)?;
                match code {
                    0 => {} // AuthenticationOk — proceed
                    3 => {
                        // AuthenticationCleartextPassword
                        let pw = require_password(cfg)?;
                        let mut msg = pw.as_bytes().to_vec();
                        msg.push(0);
                        send(stream, b'p', &msg)?;
                    }
                    5 => {
                        // AuthenticationMD5Password: body[4..8] is the 4-byte salt
                        let salt = body
                            .get(4..8)
                            .ok_or_else(|| invalid("MD5 authentication request missing salt"))?;
                        let pw = require_password(cfg)?;
                        let mut msg = pg_md5(&cfg.user, pw, salt).into_bytes();
                        msg.push(0);
                        send(stream, b'p', &msg)?;
                    }
                    10 => {
                        return Err(io::Error::new(
                            io::ErrorKind::Unsupported,
                            "SCRAM auth not supported",
                        ));
                    }
                    other => {
                        return Err(io::Error::new(
                            io::ErrorKind::Unsupported,
                            format!("unsupported authentication request {other}"),
                        ));
                    }
                }
            }
            b'E' => return Err(backend_error(&body)),
            b'Z' => return Ok(()),
            _ => {} // ParameterStatus, BackendKeyData, NoticeResponse, … — ignore
        }
    }
}

/// Fetch the configured password or fail with a clear error.
fn require_password(cfg: &PgConfig) -> io::Result<&str> {
    cfg.password.as_deref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "server requested a password but none was configured",
        )
    })
}

/// The `PostgreSQL` `MD5` password response: `"md5" + hex(md5(hex(md5(password + user)) + salt))`.
fn pg_md5(user: &str, password: &str, salt: &[u8]) -> String {
    let mut inner = password.as_bytes().to_vec();
    inner.extend_from_slice(user.as_bytes());
    let inner_hex = hex(&md5(&inner));

    let mut outer = inner_hex.into_bytes();
    outer.extend_from_slice(salt);
    let outer_hex = hex(&md5(&outer));

    format!("md5{outer_hex}")
}

// ---------------------------------------------------------------------------------------------------
// COPY text-format escaping
// ---------------------------------------------------------------------------------------------------

/// Escape a record for the `COPY` text format (backslash, tab, newline, carriage-return).
fn escape_copy(record: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(record.len());
    for &byte in record {
        match byte {
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            other => out.push(other),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------
// Identifier validation + error helpers
// ---------------------------------------------------------------------------------------------------

/// Reject an empty identifier or one containing a double-quote or NUL.
fn validate_ident(ident: &str) -> io::Result<()> {
    if ident.is_empty() {
        return Err(invalid("identifier must not be empty"));
    }
    if ident.bytes().any(|b| b == b'"' || b == 0) {
        return Err(invalid("identifier must not contain a double-quote or NUL"));
    }
    Ok(())
}

/// Parse an `ErrorResponse` body into an [`io::Error`], extracting the human-readable fields.
fn backend_error(body: &[u8]) -> io::Error {
    io::Error::other(error_message(body))
}

/// Extract a readable message from an `ErrorResponse`/`NoticeResponse` body (a series of
/// type-byte + C-string fields, terminated by a zero type byte).
fn error_message(body: &[u8]) -> String {
    let mut rest = body;
    let mut severity: Option<String> = None;
    let mut code: Option<String> = None;
    let mut message: Option<String> = None;

    while let Some((&ftype, after)) = rest.split_first() {
        if ftype == 0 {
            break;
        }
        let end = after.iter().position(|&b| b == 0).unwrap_or(after.len());
        let (value, tail) = after.split_at(end);
        let text = String::from_utf8_lossy(value).into_owned();
        match ftype {
            b'S' => severity = Some(text),
            b'C' => code = Some(text),
            b'M' => message = Some(text),
            _ => {}
        }
        rest = tail.get(1..).unwrap_or(&[]); // skip the field's NUL terminator
    }

    match (severity, message, code) {
        (Some(sev), Some(msg), Some(sqlstate)) => format!("{sev}: {msg} (SQLSTATE {sqlstate})"),
        (Some(sev), Some(msg), None) => format!("{sev}: {msg}"),
        (_, Some(msg), _) => msg,
        _ => "unknown postgres error response".to_owned(),
    }
}

fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn too_large() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "message too large to frame")
}

// ---------------------------------------------------------------------------------------------------
// Hand-rolled MD5 (RFC 1321), zero-dependency
// ---------------------------------------------------------------------------------------------------

/// Per-round left-rotate amounts.
const MD5_SHIFTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, //
    5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, //
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, //
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// Per-round additive constants: `floor(2^32 * abs(sin(i + 1)))`.
const MD5_K: [u32; 64] = [
    0xd76a_a478, 0xe8c7_b756, 0x2420_70db, 0xc1bd_ceee, 0xf57c_0faf, 0x4787_c62a, 0xa830_4613,
    0xfd46_9501, 0x6980_98d8, 0x8b44_f7af, 0xffff_5bb1, 0x895c_d7be, 0x6b90_1122, 0xfd98_7193,
    0xa679_438e, 0x49b4_0821, 0xf61e_2562, 0xc040_b340, 0x265e_5a51, 0xe9b6_c7aa, 0xd62f_105d,
    0x0244_1453, 0xd8a1_e681, 0xe7d3_fbc8, 0x21e1_cde6, 0xc337_07d6, 0xf4d5_0d87, 0x455a_14ed,
    0xa9e3_e905, 0xfcef_a3f8, 0x676f_02d9, 0x8d2a_4c8a, 0xfffa_3942, 0x8771_f681, 0x6d9d_6122,
    0xfde5_380c, 0xa4be_ea44, 0x4bde_cfa9, 0xf6bb_4b60, 0xbebf_bc70, 0x289b_7ec6, 0xeaa1_27fa,
    0xd4ef_3085, 0x0488_1d05, 0xd9d4_d039, 0xe6db_99e5, 0x1fa2_7cf8, 0xc4ac_5665, 0xf429_2244,
    0x432a_ff97, 0xab94_23a7, 0xfc93_a039, 0x655b_59c3, 0x8f0c_cc92, 0xffef_f47d, 0x8584_5dd1,
    0x6fa8_7e4f, 0xfe2c_e6e0, 0xa301_4314, 0x4e08_11a1, 0xf753_7e82, 0xbd3a_f235, 0x2ad7_d2bb,
    0xeb86_d391,
];

/// Compute the 16-byte MD5 digest of `input`.
fn md5(input: &[u8]) -> [u8; 16] {
    // Pad: append 0x80, then zeros to 56 mod 64, then the 64-bit little-endian bit length.
    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut msg = input.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xefcd_ab89;
    let mut h2: u32 = 0x98ba_dcfe;
    let mut h3: u32 = 0x1032_5476;

    for chunk in msg.chunks_exact(64) {
        let mut words = [0u32; 16];
        for (word, bytes) in words.iter_mut().zip(chunk.chunks_exact(4)) {
            *word = u32::from_le_bytes(bytes.try_into().unwrap_or([0; 4]));
        }

        let mut aa = h0;
        let mut bb = h1;
        let mut cc = h2;
        let mut dd = h3;

        for round in 0..64 {
            let (mut mix, word_idx) = if round < 16 {
                ((bb & cc) | (!bb & dd), round)
            } else if round < 32 {
                ((dd & bb) | (!dd & cc), (5 * round + 1) % 16)
            } else if round < 48 {
                (bb ^ cc ^ dd, (3 * round + 5) % 16)
            } else {
                (cc ^ (bb | !dd), (7 * round) % 16)
            };
            let kval = MD5_K.get(round).copied().unwrap_or(0);
            let shift = MD5_SHIFTS.get(round).copied().unwrap_or(0);
            let wval = words.get(word_idx).copied().unwrap_or(0);

            mix = mix
                .wrapping_add(aa)
                .wrapping_add(kval)
                .wrapping_add(wval);
            aa = dd;
            dd = cc;
            cc = bb;
            bb = bb.wrapping_add(mix.rotate_left(shift));
        }

        h0 = h0.wrapping_add(aa);
        h1 = h1.wrapping_add(bb);
        h2 = h2.wrapping_add(cc);
        h3 = h3.wrapping_add(dd);
    }

    let mut out = [0u8; 16];
    for (dst, val) in out.chunks_exact_mut(4).zip([h0, h1, h2, h3]) {
        dst.copy_from_slice(&val.to_le_bytes());
    }
    out
}

/// Lowercase hex encoding of `bytes`.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = Vec::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(DIGITS.get(usize::from(b >> 4)).copied().unwrap_or(b'?'));
        out.push(DIGITS.get(usize::from(b & 0x0f)).copied().unwrap_or(b'?'));
    }
    String::from_utf8(out).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{
        escape_copy, frame_bytes, hex, md5, pg_md5, read_msg, startup_bytes, validate_ident,
    };
    use std::io::Cursor;

    #[test]
    fn md5_known_answers() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        // A multi-block input (> 64 bytes) exercises the chunk loop and padding.
        assert_eq!(
            hex(&md5(
                b"The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs."
            )),
            "f4188739a916382a9b8b684dea92ac8d",
        );
    }

    #[test]
    fn pg_md5_full_digest_matches_reference() {
        // Reference computed independently: user="alice", password="secret", salt=01 02 03 04.
        let digest = pg_md5("alice", "secret", &[0x01, 0x02, 0x03, 0x04]);
        assert_eq!(digest, "md598a0412b9c31436fc53776e863350083");
    }

    #[test]
    fn startup_message_exact_bytes() {
        let got = startup_bytes("u", "d").expect("startup");
        let mut want = Vec::new();
        want.extend_from_slice(&27i32.to_be_bytes()); // total length
        want.extend_from_slice(&196_608i32.to_be_bytes()); // protocol
        want.extend_from_slice(b"user\0u\0database\0d\0\0");
        assert_eq!(got, want);
    }

    #[test]
    fn copy_data_frame_exact_bytes() {
        // record "ab" -> escaped "ab" + row terminator '\n' = 3 body bytes; total = 7.
        let mut body = escape_copy(b"ab");
        body.push(b'\n');
        let got = frame_bytes(b'd', &body).expect("frame");
        assert_eq!(got, vec![b'd', 0, 0, 0, 7, b'a', b'b', b'\n']);
    }

    #[test]
    fn copy_text_escaping() {
        // tab, newline, backslash, carriage-return each get a backslash escape.
        let got = escape_copy(b"a\tb\nc\\d\re");
        assert_eq!(got, b"a\\tb\\nc\\\\d\\re".to_vec());
    }

    #[test]
    fn read_msg_rejects_truncated_header() {
        // Only 3 bytes: cannot even read the 5-byte header -> Err, not panic.
        let mut cur = Cursor::new(vec![b'X', 0, 0]);
        assert!(read_msg(&mut cur).is_err());
    }

    #[test]
    fn read_msg_rejects_undersized_length() {
        // Valid 5-byte header but length field < 4 -> Err.
        let mut cur = Cursor::new(vec![b'E', 0, 0, 0, 3]);
        assert!(read_msg(&mut cur).is_err());
    }

    #[test]
    fn read_msg_parses_well_formed_message() {
        // tag 'C', length 6 (covers length + 2 body bytes), body "ok".
        let mut cur = Cursor::new(vec![b'C', 0, 0, 0, 6, b'o', b'k']);
        let (tag, body) = read_msg(&mut cur).expect("read");
        assert_eq!(tag, b'C');
        assert_eq!(body, b"ok".to_vec());
    }

    #[test]
    fn validate_ident_rejects_quote_and_nul() {
        assert!(validate_ident("events").is_ok());
        assert!(validate_ident("bad\"name").is_err());
        assert!(validate_ident("bad\0name").is_err());
        assert!(validate_ident("").is_err());
    }
}
