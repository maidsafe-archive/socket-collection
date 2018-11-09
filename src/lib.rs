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
extern crate libc;
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
mod out_queue;
mod tcp_sock;
mod udp;
#[cfg(target_os = "linux")]
mod utils;

#[cfg(feature = "enable-udt")]
mod udt;

/// Priority of a message to be sent by Crust. A lower value means a higher priority, so Priority 0
/// is the highest one. Low-priority messages will be preempted if need be to allow higher priority
/// messages through. Messages with a value `>= MSG_DROP_PRIORITY` will even be dropped, if
/// bandwidth is insufficient.
pub type Priority = u8;

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
    #[cfg(target_os = "linux")]
    /// All 3 configurable TCP keep alive parameters:
    ///
    /// 1. TCP_KEEPIDLE - wait for n inactivity seconds before start sending keep alive requests
    /// 2. TCP_KEEPINTVL - send keep alive packet every n seconds
    /// 3. TCP_KEEPCNT - terminate connection if no response is recevied after this count of keep
    ///                  alive packets was sent.
    ///
    /// By default keep alive is not set.
    pub keep_alive: Option<(u32, u32, u32)>,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            max_payload_size: 2 * 1024 * 1024,
            msg_drop_priority: 2,
            max_msg_age_secs: 60,
            #[cfg(target_os = "linux")]
            keep_alive: None,
        }
    }
}

/// `Result` type specialised for this crate
pub type Res<T> = Result<T, SocketError>;
