// Copyright 2018 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

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

pub use crate::crypto::{DecryptContext, EncryptContext};
pub use crate::error::SocketError;
pub use crate::socket::Socket;
pub use crate::tcp::TcpSock;
pub use crate::udp::UdpSock;

// #[cfg(feature = "enable-udt")]
// pub use udt::{EpollLoop, Handle, Notifier, UdtSock};

mod crypto;
mod error;
mod out_queue;
mod socket;
mod tcp;
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
