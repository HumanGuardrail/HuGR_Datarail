//! `datarail-netblob` — FROZEN API (frozen by the lead); WP3 implements the bodies + gates.
//! Serves any [`datarail_blobstore::BlobStore`] over a TCP socket, and provides `NetBlob` — a client that
//! implements `BlobStore` by round-tripping put/get/list/delete to that server (the cold tier, made remote).
#![forbid(unsafe_code)]
use datarail_blobstore::BlobStore;

/// A client `BlobStore` that forwards every op to a remote [`serve`] endpoint over TCP.
pub struct NetBlob {
    addr: std::net::SocketAddr,
}

impl NetBlob {
    /// Connect to a blob server at `addr`.
    #[must_use]
    pub fn new(addr: std::net::SocketAddr) -> Self {
        Self { addr }
    }
    /// The server address.
    #[must_use]
    pub fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }
}

impl BlobStore for NetBlob {
    fn put(&mut self, _key: &str, _bytes: &[u8]) -> Result<(), std::io::Error> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "WP3: unimplemented"))
    }
    fn get(&self, _key: &str) -> Result<Option<Vec<u8>>, std::io::Error> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "WP3: unimplemented"))
    }
    fn list(&self, _prefix: &str) -> Result<Vec<String>, std::io::Error> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "WP3: unimplemented"))
    }
    fn delete(&mut self, _key: &str) -> Result<bool, std::io::Error> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "WP3: unimplemented"))
    }
}

/// Serve `store` on a fresh TCP listener; returns the bound address + a handle. WP3: implement (spawn a thread
/// that accepts connections and applies each framed op to `store`). The signature is the frozen contract.
/// # Errors
/// Bind failure.
pub fn serve<B: BlobStore + Send + 'static>(_store: B) -> Result<std::net::SocketAddr, std::io::Error> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "WP3: unimplemented"))
}
