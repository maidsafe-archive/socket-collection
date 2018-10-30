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

extern crate byteorder;
extern crate maidsafe_utilities;
extern crate mio;
extern crate serde;

#[cfg(feature = "enable-udt")]
extern crate libudt4_sys;
#[cfg(feature = "enable-udt")]
extern crate udt as udt_extern;

pub use error::SocketError;
pub use tcp_sock::TcpSock;
pub use udp::UdpSock;

#[cfg(feature = "enable-udt")]
pub use udt::{EpollLoop, Handle, Notifier, UdtSock};

mod error;
mod tcp_sock;
mod udp;

#[cfg(feature = "enable-udt")]
mod udt;

/// Priority of a message to be sent by Crust. A lower value means a higher priority, so Priority 0
/// is the highest one. Low-priority messages will be preempted if need be to allow higher priority
/// messages through. Messages with a value `>= MSG_DROP_PRIORITY` will even be dropped, if
/// bandwidth is insufficient.
pub type Priority = u8;

pub const MAX_PAYLOAD_SIZE: usize = 2 * 1024 * 1024;
/// Minimum priority for droppable messages. Messages with lower values will never be dropped.
pub const MSG_DROP_PRIORITY: u8 = 2;
/// Maximum age of a message waiting to be sent. If a message is older, the queue is dropped.
pub const MAX_MSG_AGE_SECS: u64 = 60;
/// `Result` type specialised for this crate
pub type Res<T> = Result<T, SocketError>;
