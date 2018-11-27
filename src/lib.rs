//! # Overview
//!
//! The goal of this crate is to provide a collection of async sockets which can be used out of the
//! box with `mio` event loop. As a simple example, using stream based protocols will require some
//! sort of mechanism to determine the boundaries of a message etc., and this crate provides default
//! implementation to handle those and abstract the boilerplate from the user libs.

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
extern crate byteorder;
extern crate maidsafe_utilities;
extern crate mio;
extern crate safe_crypto;
extern crate serde;

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
