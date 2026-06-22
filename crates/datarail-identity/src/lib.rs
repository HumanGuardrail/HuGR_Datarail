//! `datarail-identity` — endpoint identity & pairing for the rail, per the frozen SPEC `08`.
//!
//! Two independent building blocks live here:
//!
//! - [`noise`] (**F2**) — a `Noise_KK` channel session built on the [`snow`] crate. `KK` means **both**
//!   static keys are pinned a priori, which matches datarail's fixed `A -> B` route model: each endpoint
//!   knows the other's long-term static public key before the handshake starts. Completing the 2-message
//!   handshake proves **mutual** possession of the matching private keys (channel-level mutual auth) and
//!   yields a **forward-secret** transport session for a substrate hop.
//! - [`pairing`] (**F3**) — a short-code bootstrap built on the [`spake2`] PAKE (a balanced PAKE; the SPEC
//!   forbids hand-rolling one — see the croc `CVE-2021-31603` note). It turns a high-entropy, single-use
//!   code into a shared secret while **binding both endpoints' static public keys into the transcript**, so
//!   success authenticates *which* identities paired and a man-in-the-middle cannot substitute its own key.
//!
//! ## Scope boundary (important)
//!
//! Neither block replaces the **per-cofre seal**. A cofre is sealed end-to-end by `datarail-terminal` /
//! `datarail-crypto` independently of any channel: the substrate — and therefore any `Noise_KK` session over
//! it — only ever moves *already-sealed* ciphertext. `Noise_KK` authenticates the two **endpoints** and can
//! protect a substrate hop; the PAKE bootstrap is how two endpoints first agree on each other's pinned static
//! keys out-of-band. The seal's confidentiality does not depend on either.
//!
//! No `unsafe`, no panics in library code.
#![forbid(unsafe_code)]

pub mod noise;
pub mod noise_substrate;
pub mod pairing;

pub use noise::{KkSession, NoiseError, StaticKeypair, Transport};
pub use noise_substrate::NoiseSubstrate;
pub use pairing::{Pairing, PairingError, PairingOutcome, ShortCode};
