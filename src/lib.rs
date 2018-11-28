//! # Overview
//!
//! The goal of this crate is to provide a collection of async sockets which can be used out of the
//! box with `mio` event loop. Currently the crate exposes UDP and TCP sockets: [`UdpSock`] and
//! [`TcpSock`] respectively. The socket behavior is very specific for [`p2p`] and [`Crust`]
//! crates. We aim to make the sockets easy to use and reduce boilerplate code. The sockets buffer
//! incoming/outgoing data, implement message serialization and encryption, the user of the
//! stream based sockets don't need to worry about message boundaries, each message has a priority,
//! number etc.
//!
//! [`UdpSock`]: struct.UdpSock.html
//! [`TcpSock`]: struct.TcpSock.html
//! [`p2p`]: https://crates.io/crates/p2p
//! [`Crust`]: https://crates.io/crates/crust

#[macro_use]
extern crate log;
#[cfg_attr(feature = "cargo-clippy", allow(useless_attribute))]
#[macro_use]
extern crate quick_error;
#[macro_use]
extern crate unwrap;
#[cfg(test)]
#[macro_use]
extern crate proptest;
extern crate maidsafe_utilities;
extern crate mio;
extern crate safe_crypto;
extern crate serde;
#[cfg(test)]
#[macro_use]
extern crate hamcrest2;

// #[cfg(feature = "enable-udt")]
// extern crate libudt4_sys;
// #[cfg(feature = "enable-udt")]
// extern crate udt as udt_extern;

pub use crypto::{DecryptContext, EncryptContext};
pub use error::SocketError;
pub use tcp_sock::TcpSock;
pub use udp::UdpSock;

// #[cfg(feature = "enable-udt")]
// pub use udt::{EpollLoop, Handle, Notifier, UdtSock};

mod crypto;
mod error;
mod out_queue;
mod tcp_sock;
mod udp;

use priv_prelude::*;

// #[cfg(feature = "enable-udt")]
// mod udt;

/// Priority of a message to be sent by Crust. A lower value means a higher priority, so Priority 0
/// is the highest one. Low-priority messages will be preempted if need be to allow higher priority
/// messages through. Messages with a value `>= MSG_DROP_PRIORITY` will even be dropped, if
/// bandwidth is insufficient.
pub type Priority = u8;

/// Don't allow packets bigger than this value.
pub const DEFAULT_MAX_PAYLOAD_SIZE: usize = 2 * 1024 * 1024;
/// Minimum priority for droppable messages. Messages with lower values will never be dropped.
pub const DEFAULT_MSG_DROP_PRIORITY: u8 = 2;
/// Maximum age of a message waiting to be sent. If a message is older, the queue is dropped.
pub const DEFAULT_MAX_MSG_AGE_SECS: u64 = 60;

/// Configures socket behavior.
#[derive(Debug, Clone)]
pub struct SocketConfig {
    /// Maximum data size that the socket will send.
    /// Nonaplicable for UDP socket whose max message size is determined by max UDP payload size.
    pub max_payload_size: usize,
    /// Minimum priority for droppable messages. Messages with lower values will never be dropped.
    pub msg_drop_priority: u8,
    /// Maximum age of a message waiting to be sent. If a message is older, the queue is dropped.
    pub max_msg_age_secs: u64,
    /// Data that goes throught socket encryption scheme.
    pub enc_ctx: EncryptContext,
    /// Data that goes throught socket decryption scheme.
    pub dec_ctx: DecryptContext,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            max_payload_size: DEFAULT_MAX_PAYLOAD_SIZE,
            msg_drop_priority: DEFAULT_MSG_DROP_PRIORITY,
            max_msg_age_secs: DEFAULT_MAX_MSG_AGE_SECS,
            enc_ctx: Default::default(),
            dec_ctx: Default::default(),
        }
    }
}

/// `Result` type specialised for this crate
pub type Res<T> = Result<T, SocketError>;

/// Protocol agnostic socket trait. It is implemented for [`UdpSock`] and [`TcpSock`].
///
/// [`UdpSock`]: struct.UdpSock.html
/// [`TcpSock`]: struct.TcpSock.html
pub trait Socket {
    /// The underlying mio socket type that structs implementing `Socket` trait wrap.
    type Inner;

    /// Specify data encryption context which will determine how outgoing data is encrypted.
    fn set_encrypt_ctx(&mut self, enc_ctx: EncryptContext) -> ::Res<()>;

    /// Specify data decryption context which will determine how incoming data is decrypted.
    fn set_decrypt_ctx(&mut self, dec_ctx: DecryptContext) -> ::Res<()>;

    /// Get the local address socket is bound to.
    fn local_addr(&self) -> ::Res<SocketAddr>;

    /// Get the address socket was connected to.
    fn peer_addr(&self) -> ::Res<SocketAddr>;

    /// Set Time To Live value for the underlying socket.
    fn set_ttl(&self, ttl: u32) -> ::Res<()>;

    /// Retrieve Time To Live value.
    fn ttl(&self) -> ::Res<u32>;

    /// Retrieve last socket error, if one exists.
    fn take_error(&self) -> ::Res<Option<std::io::Error>>;

    /// Read message from the connected socket. Call this method once socket becomes readable.
    ///
    /// # Returns:
    ///
    ///   - Ok(Some(data)): data has been successfully read from the socket
    ///   - Ok(None):       there is not enough data in the socket. Call `read()`
    ///                     again in the next invocation of the `ready` handler.
    ///   - Err(error):     there was an error reading from the socket.
    fn read<T: DeserializeOwned + Serialize>(&mut self) -> ::Res<Option<T>>;

    /// Write a message to the connected socket.
    ///
    /// # Returns:
    ///
    ///   - Ok(true):   the message has been successfully written.
    ///   - Ok(false):  the message has been queued, but not yet fully written.
    ///                 will be attempted in the next write schedule.
    ///   - Err(error): there was an error while writing to the socket.
    fn write<T: Serialize>(&mut self, msg: Option<(T, Priority)>) -> ::Res<bool>;

    /// Sets linger time for connection based protocols.
    fn set_linger(&self, dur: Option<Duration>) -> ::Res<()>;

    /// Return the wrapped mio socket.
    fn into_underlying_sock(self) -> ::Res<Self::Inner>;
}
