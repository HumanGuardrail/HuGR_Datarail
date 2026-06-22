//! **F2** — `Noise_KK` channel session (SPEC `08`).
//!
//! `Noise_KK_25519_ChaChaPoly_BLAKE2s` over the [`snow`] crate. The handshake is **never** hand-rolled; this
//! module only drives `snow`'s state machine and pins the right keys.
//!
//! ## What `KK` buys us
//!
//! In the `KK` pattern both parties pin the *other's* static public key **before** the handshake (`K` =
//! "known" on each side). That is exactly datarail's model: a route `A -> B` is fixed, and each endpoint is
//! provisioned with the peer's long-term static key (e.g. via the [`crate::pairing`] bootstrap). Driving the
//! two messages to completion proves each side holds the private key for the static key the other pinned —
//! i.e. **mutual authentication** — and derives a fresh, **forward-secret** session from the ephemeral DH.
//!
//! ## What it is *not*
//!
//! This authenticates the **channel endpoints** and can protect a substrate hop. It does **not** seal the
//! cofre: the cofre is sealed end-to-end elsewhere, and only sealed ciphertext is ever fed through a session
//! here. See the crate-level docs.
//!
//! ## Usage
//!
//! ```
//! use datarail_identity::noise::{KkSession, StaticKeypair};
//!
//! # fn main() -> Result<(), datarail_identity::noise::NoiseError> {
//! // Each endpoint has a long-term static keypair and has pinned the peer's static *public* key.
//! let a = StaticKeypair::generate()?;
//! let b = StaticKeypair::generate()?;
//!
//! let mut initiator = KkSession::initiator(&a, b.public())?;
//! let mut responder = KkSession::responder(&b, a.public())?;
//!
//! // -> e, es, ss
//! let msg1 = initiator.write_handshake(&[])?;
//! responder.read_handshake(&msg1)?;
//! // <- e, ee, se
//! let msg2 = responder.write_handshake(&[])?;
//! initiator.read_handshake(&msg2)?;
//!
//! // Handshake complete on both ends; move to the forward-secret transport.
//! let mut at = initiator.into_transport()?;
//! let mut bt = responder.into_transport()?;
//!
//! let ct = at.encrypt(b"sealed-cofre-bytes")?;
//! assert_eq!(bt.decrypt(&ct)?, b"sealed-cofre-bytes");
//! # Ok(())
//! # }
//! ```

use snow::{Builder, HandshakeState, TransportState};

/// The frozen handshake pattern for datarail channel sessions (SPEC `08` F2).
const PATTERN: &str = "Noise_KK_25519_ChaChaPoly_BLAKE2s";

/// Length of an X25519 public key, in bytes.
pub const STATIC_PUBLIC_LEN: usize = 32;

/// Maximum length of a single Noise message buffer, per the Noise spec (`65535` bytes on the wire).
const MAX_MESSAGE_LEN: usize = 65535;

/// Errors from the `Noise_KK` session (F2).
#[derive(Debug)]
#[non_exhaustive]
pub enum NoiseError {
    /// The underlying `snow` engine reported an error (e.g. authentication failure on a forged static key,
    /// a malformed handshake message, or a decryption tag mismatch).
    Engine(snow::Error),
    /// A handshake step was requested in the wrong order, or after the handshake already finished.
    WrongState,
    /// A message exceeded the Noise transport limit of [`MAX_MESSAGE_LEN`] bytes.
    MessageTooLong,
}

impl core::fmt::Display for NoiseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Engine(e) => write!(f, "noise engine error: {e}"),
            Self::WrongState => f.write_str("noise handshake step out of order"),
            Self::MessageTooLong => f.write_str("message exceeds the noise transport limit"),
        }
    }
}

impl std::error::Error for NoiseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Engine(e) => Some(e),
            Self::WrongState | Self::MessageTooLong => None,
        }
    }
}

impl From<snow::Error> for NoiseError {
    fn from(e: snow::Error) -> Self {
        Self::Engine(e)
    }
}

/// A long-term static X25519 keypair for one endpoint.
///
/// The **private** half stays on the endpoint and is fed into the local handshake; the **public** half is
/// what a peer pins (via [`public`](Self::public)) to authenticate this endpoint in the `KK` pattern.
#[derive(Clone)]
pub struct StaticKeypair {
    private: Vec<u8>,
    public: [u8; STATIC_PUBLIC_LEN],
}

impl StaticKeypair {
    /// Generate a fresh static keypair using `snow`'s configured CSPRNG.
    ///
    /// # Errors
    ///
    /// Returns [`NoiseError::Engine`] if the cryptographic backend fails to produce a keypair (e.g. the OS
    /// entropy source is unavailable) or if the frozen [`PATTERN`] fails to parse.
    pub fn generate() -> Result<Self, NoiseError> {
        let params = PATTERN.parse()?;
        let kp = Builder::new(params).generate_keypair()?;
        let mut public = [0u8; STATIC_PUBLIC_LEN];
        // `snow` returns X25519 keys, which are exactly `STATIC_PUBLIC_LEN` bytes.
        if kp.public.len() != STATIC_PUBLIC_LEN {
            return Err(NoiseError::Engine(snow::Error::Dh));
        }
        public.copy_from_slice(&kp.public);
        Ok(Self {
            private: kp.private,
            public,
        })
    }

    /// Reconstruct a static keypair from a persisted 32-byte X25519 **secret** — so an endpoint keeps a
    /// **stable** identity across runs (a peer pins the matching [`public`](Self::public)). The public is
    /// derived with standard X25519 ([`datarail_crypto::x25519_public`]), which agrees with what `snow` derives
    /// for the same secret during the handshake (proven by `from_secret_keypairs_complete_a_handshake`).
    #[must_use]
    pub fn from_secret(secret: [u8; STATIC_PUBLIC_LEN]) -> Self {
        let public = datarail_crypto::x25519_public(&secret);
        Self {
            private: secret.to_vec(),
            public,
        }
    }

    /// This endpoint's static **secret** key (32 bytes) — persist it to keep a stable identity; treat as
    /// secret. (`StaticKeypair` has no `Debug`, so it cannot be printed accidentally.)
    #[must_use]
    pub fn secret(&self) -> [u8; STATIC_PUBLIC_LEN] {
        let mut s = [0u8; STATIC_PUBLIC_LEN];
        let n = self.private.len().min(STATIC_PUBLIC_LEN);
        s[..n].copy_from_slice(&self.private[..n]);
        s
    }

    /// This endpoint's static **public** key — the value a peer must pin to authenticate this endpoint.
    #[must_use]
    pub fn public(&self) -> [u8; STATIC_PUBLIC_LEN] {
        self.public
    }
}

/// One side of an in-progress `Noise_KK` handshake.
///
/// Built with [`initiator`](Self::initiator) or [`responder`](Self::responder), driven with
/// [`write_handshake`](Self::write_handshake) / [`read_handshake`](Self::read_handshake), then converted to a
/// [`Transport`] with [`into_transport`](Self::into_transport) once the handshake is finished.
pub struct KkSession {
    state: HandshakeState,
}

impl KkSession {
    /// Build the **initiator** of an `A -> B` session: `local` is this endpoint's static keypair and
    /// `remote_static_public` is the peer's pinned static public key.
    ///
    /// # Errors
    ///
    /// Returns [`NoiseError`] if the pattern fails to parse or `snow` rejects the supplied keys.
    pub fn initiator(
        local: &StaticKeypair,
        remote_static_public: [u8; STATIC_PUBLIC_LEN],
    ) -> Result<Self, NoiseError> {
        Self::build(local, remote_static_public, Role::Initiator)
    }

    /// Build the **responder** of an `A -> B` session: `local` is this endpoint's static keypair and
    /// `remote_static_public` is the initiator's pinned static public key.
    ///
    /// # Errors
    ///
    /// Returns [`NoiseError`] if the pattern fails to parse or `snow` rejects the supplied keys.
    pub fn responder(
        local: &StaticKeypair,
        remote_static_public: [u8; STATIC_PUBLIC_LEN],
    ) -> Result<Self, NoiseError> {
        Self::build(local, remote_static_public, Role::Responder)
    }

    fn build(
        local: &StaticKeypair,
        remote_static_public: [u8; STATIC_PUBLIC_LEN],
        role: Role,
    ) -> Result<Self, NoiseError> {
        let params = PATTERN.parse()?;
        // In snow 0.9, `local_private_key` / `remote_public_key` are infallible builder setters returning
        // `Self`; only `build_*` returns `Result`.
        let builder = Builder::new(params)
            .local_private_key(&local.private)
            .remote_public_key(&remote_static_public);
        let state = match role {
            Role::Initiator => builder.build_initiator()?,
            Role::Responder => builder.build_responder()?,
        };
        Ok(Self { state })
    }

    /// Write the next handshake message (with optional `payload`) into a fresh buffer and return it.
    ///
    /// Call this only when it is this side's turn to send.
    ///
    /// # Errors
    ///
    /// Returns [`NoiseError::WrongState`] if the handshake is already finished, [`NoiseError::MessageTooLong`]
    /// if `payload` would overflow the Noise message limit, or [`NoiseError::Engine`] on a `snow` failure.
    pub fn write_handshake(&mut self, payload: &[u8]) -> Result<Vec<u8>, NoiseError> {
        if self.state.is_handshake_finished() {
            return Err(NoiseError::WrongState);
        }
        if payload.len() > MAX_MESSAGE_LEN {
            return Err(NoiseError::MessageTooLong);
        }
        let mut buf = vec![0u8; MAX_MESSAGE_LEN];
        let len = self.state.write_message(payload, &mut buf)?;
        buf.truncate(len);
        Ok(buf)
    }

    /// Read an incoming handshake `message`, returning any decrypted payload it carried.
    ///
    /// Call this only when it is this side's turn to receive.
    ///
    /// # Errors
    ///
    /// Returns [`NoiseError::WrongState`] if the handshake is already finished, [`NoiseError::MessageTooLong`]
    /// if `message` exceeds the Noise limit, or [`NoiseError::Engine`] if authentication/decryption fails —
    /// which is exactly what happens when a peer presents the wrong pinned static key (mutual-auth failure).
    pub fn read_handshake(&mut self, message: &[u8]) -> Result<Vec<u8>, NoiseError> {
        if self.state.is_handshake_finished() {
            return Err(NoiseError::WrongState);
        }
        if message.len() > MAX_MESSAGE_LEN {
            return Err(NoiseError::MessageTooLong);
        }
        let mut buf = vec![0u8; MAX_MESSAGE_LEN];
        let len = self.state.read_message(message, &mut buf)?;
        buf.truncate(len);
        Ok(buf)
    }

    /// Whether the handshake has completed and the session is ready for [`into_transport`](Self::into_transport).
    #[must_use]
    pub fn is_handshake_finished(&self) -> bool {
        self.state.is_handshake_finished()
    }

    /// The peer's static public key as seen by `snow` after the handshake (a cross-check against the pinned
    /// value). `None` before it is known.
    #[must_use]
    pub fn remote_static(&self) -> Option<Vec<u8>> {
        self.state.get_remote_static().map(<[u8]>::to_vec)
    }

    /// Consume the finished handshake and return the forward-secret [`Transport`] session.
    ///
    /// # Errors
    ///
    /// Returns [`NoiseError::WrongState`] if the handshake has not finished, or [`NoiseError::Engine`] if
    /// `snow` cannot split the session keys.
    pub fn into_transport(self) -> Result<Transport, NoiseError> {
        if !self.state.is_handshake_finished() {
            return Err(NoiseError::WrongState);
        }
        let state = self.state.into_transport_mode()?;
        Ok(Transport { state })
    }
}

/// An established, forward-secret `Noise_KK` transport session.
///
/// Each [`encrypt`](Self::encrypt) / [`decrypt`](Self::decrypt) advances `snow`'s internal nonce counter, so
/// messages are authenticated and ordered. Use it to protect a substrate hop carrying sealed cofre bytes.
pub struct Transport {
    state: TransportState,
}

impl Transport {
    /// Encrypt `plaintext` into a fresh ciphertext buffer (AEAD: `ChaChaPoly`).
    ///
    /// # Errors
    ///
    /// Returns [`NoiseError::MessageTooLong`] if `plaintext` is too large for one Noise message, or
    /// [`NoiseError::Engine`] on a `snow` failure.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, NoiseError> {
        if plaintext.len() > MAX_MESSAGE_LEN {
            return Err(NoiseError::MessageTooLong);
        }
        let mut buf = vec![0u8; MAX_MESSAGE_LEN];
        let len = self.state.write_message(plaintext, &mut buf)?;
        buf.truncate(len);
        Ok(buf)
    }

    /// Decrypt and authenticate `ciphertext`, returning the recovered plaintext.
    ///
    /// # Errors
    ///
    /// Returns [`NoiseError::MessageTooLong`] if `ciphertext` exceeds the Noise limit, or
    /// [`NoiseError::Engine`] if the AEAD tag fails to verify (tampered or out-of-order message).
    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, NoiseError> {
        if ciphertext.len() > MAX_MESSAGE_LEN {
            return Err(NoiseError::MessageTooLong);
        }
        let mut buf = vec![0u8; MAX_MESSAGE_LEN];
        let len = self.state.read_message(ciphertext, &mut buf)?;
        buf.truncate(len);
        Ok(buf)
    }
}

/// Which end of the `KK` handshake a [`KkSession`] is.
#[derive(Clone, Copy)]
enum Role {
    Initiator,
    Responder,
}

#[cfg(test)]
mod tests {
    use super::{KkSession, StaticKeypair};

    /// Drive a full `KK` handshake between an initiator and a responder, returning both transports.
    fn complete_handshake(
        a: &StaticKeypair,
        a_peer_pin: [u8; 32],
        b: &StaticKeypair,
        b_peer_pin: [u8; 32],
    ) -> Result<(super::Transport, super::Transport), super::NoiseError> {
        let mut initiator = KkSession::initiator(a, a_peer_pin)?;
        let mut responder = KkSession::responder(b, b_peer_pin)?;

        let msg1 = initiator.write_handshake(&[])?;
        responder.read_handshake(&msg1)?;
        let msg2 = responder.write_handshake(&[])?;
        initiator.read_handshake(&msg2)?;

        assert!(initiator.is_handshake_finished());
        assert!(responder.is_handshake_finished());

        Ok((initiator.into_transport()?, responder.into_transport()?))
    }

    // from_secret derives the SAME public snow uses for that secret: two stable keypairs handshake + a
    // message round-trips. This proves datarail_crypto's X25519 public derivation is snow-compatible.
    #[test]
    fn from_secret_keypairs_complete_a_handshake() {
        let a = StaticKeypair::from_secret([0x11; 32]);
        let b = StaticKeypair::from_secret([0x22; 32]);
        assert_eq!(a.secret(), [0x11; 32], "secret round-trips");
        let (mut at, mut bt) =
            complete_handshake(&a, b.public(), &b, a.public()).expect("from_secret handshake");
        let ct = at.encrypt(b"stable-identity").unwrap();
        assert_eq!(bt.decrypt(&ct).unwrap(), b"stable-identity");
    }

    // Test (1): a correct KK handshake completes and a message round-trips equal.
    #[test]
    fn handshake_completes_and_message_round_trips() {
        let a = StaticKeypair::generate().unwrap();
        let b = StaticKeypair::generate().unwrap();

        let (mut at, mut bt) =
            complete_handshake(&a, b.public(), &b, a.public()).expect("handshake should succeed");

        // initiator -> responder
        let ct = at.encrypt(b"hello-from-A").unwrap();
        assert_eq!(bt.decrypt(&ct).unwrap(), b"hello-from-A");

        // responder -> initiator
        let ct2 = bt.encrypt(b"hello-from-B").unwrap();
        assert_eq!(at.decrypt(&ct2).unwrap(), b"hello-from-B");
    }

    // Test (2): mutual auth — a responder that pins the WRONG initiator static key (an impostor pin) must
    // fail the handshake. In KK each side authenticates the peer's pinned static key, so a mismatch is a
    // cryptographic authentication failure, not a silent success.
    #[test]
    fn impostor_remote_static_fails_handshake() {
        let a = StaticKeypair::generate().unwrap();
        let b = StaticKeypair::generate().unwrap();
        let impostor = StaticKeypair::generate().unwrap();

        // `a` is honest and pins `b`. `b` (responder) pins the IMPOSTOR's key instead of `a`'s.
        let mut initiator = KkSession::initiator(&a, b.public()).unwrap();
        let mut responder = KkSession::responder(&b, impostor.public()).unwrap();

        let msg1 = initiator.write_handshake(&[]).unwrap();
        // The responder mixes its (wrong) pinned key into the handshake; reading msg1 must fail auth.
        let read = responder.read_handshake(&msg1);
        assert!(
            read.is_err(),
            "responder pinned an impostor key; the KK handshake must fail to authenticate"
        );
    }

    // Belt-and-suspenders: the symmetric impostor case (initiator pins the wrong responder key) also fails,
    // and it fails by the time the initiator processes the response.
    #[test]
    fn initiator_pinning_impostor_fails() {
        let a = StaticKeypair::generate().unwrap();
        let b = StaticKeypair::generate().unwrap();
        let impostor = StaticKeypair::generate().unwrap();

        let mut initiator = KkSession::initiator(&a, impostor.public()).unwrap();
        let mut responder = KkSession::responder(&b, a.public()).unwrap();

        let msg1 = initiator.write_handshake(&[]).unwrap();
        // Responder may or may not reject msg1 depending on message ordering; the handshake as a whole must
        // not succeed end-to-end.
        let r1 = responder.read_handshake(&msg1);
        let end_to_end_ok = r1.is_ok()
            && responder
                .write_handshake(&[])
                .and_then(|m2| initiator.read_handshake(&m2))
                .is_ok();
        assert!(
            !end_to_end_ok,
            "initiator pinned an impostor responder key; the session must not establish"
        );
    }
}
