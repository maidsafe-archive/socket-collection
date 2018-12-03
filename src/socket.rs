use crypto::{DecryptContext, EncryptContext};
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;
use tcp_sock::TcpSock;
use udp::UdpSock;
use Priority;

/// Protocol agnostic socket that wraps [`UdpSock`] and [`TcpSock`].
///
/// [`UdpSock`]: struct.UdpSock.html
/// [`TcpSock`]: struct.TcpSock.html
#[cfg_attr(feature = "cargo-clippy", allow(large_enum_variant))]
pub enum Socket {
    Udp(UdpSock),
    Tcp(TcpSock),
}

impl Socket {
    /// Specify data encryption context which will determine how outgoing data is encrypted.
    pub fn set_encrypt_ctx(&mut self, enc_ctx: EncryptContext) -> ::Res<()> {
        match *self {
            Socket::Udp(ref mut sock) => sock.set_encrypt_ctx(enc_ctx),
            Socket::Tcp(ref mut sock) => sock.set_encrypt_ctx(enc_ctx),
        }
    }

    /// Specify data decryption context which will determine how incoming data is decrypted.
    pub fn set_decrypt_ctx(&mut self, dec_ctx: DecryptContext) -> ::Res<()> {
        match *self {
            Socket::Udp(ref mut sock) => sock.set_decrypt_ctx(dec_ctx),
            Socket::Tcp(ref mut sock) => sock.set_decrypt_ctx(dec_ctx),
        }
    }

    /// Get the local address socket is bound to.
    pub fn local_addr(&self) -> ::Res<SocketAddr> {
        match *self {
            Socket::Udp(ref sock) => sock.local_addr(),
            Socket::Tcp(ref sock) => sock.local_addr(),
        }
    }

    /// Get the address socket was connected to.
    pub fn peer_addr(&self) -> ::Res<SocketAddr> {
        match *self {
            Socket::Udp(ref sock) => sock.peer_addr(),
            Socket::Tcp(ref sock) => sock.peer_addr(),
        }
    }

    /// Set Time To Live value for the underlying socket.
    pub fn set_ttl(&self, ttl: u32) -> ::Res<()> {
        match *self {
            Socket::Udp(ref sock) => sock.set_ttl(ttl),
            Socket::Tcp(ref sock) => sock.set_ttl(ttl),
        }
    }

    /// Retrieve Time To Live value.
    pub fn ttl(&self) -> ::Res<u32> {
        match *self {
            Socket::Udp(ref sock) => sock.ttl(),
            Socket::Tcp(ref sock) => sock.ttl(),
        }
    }

    /// Retrieve last socket error, if one exists.
    pub fn take_error(&self) -> ::Res<Option<io::Error>> {
        match *self {
            Socket::Udp(ref sock) => sock.take_error(),
            Socket::Tcp(ref sock) => sock.take_error(),
        }
    }

    /// Read message from the connected socket. Call this method once socket becomes readable.
    ///
    /// # Returns:
    ///
    ///   - Ok(Some(data)): data has been successfully read from the socket
    ///   - Ok(None):       there is not enough data in the socket. Call `read()`
    ///                     again in the next invocation of the `ready` handler.
    ///   - Err(error):     there was an error reading from the socket.
    pub fn read<T: DeserializeOwned + Serialize>(&mut self) -> ::Res<Option<T>> {
        match *self {
            Socket::Udp(ref mut sock) => sock.read(),
            Socket::Tcp(ref mut sock) => sock.read(),
        }
    }

    /// Write a message to the connected socket.
    ///
    /// # Returns:
    ///
    ///   - Ok(true):   the message has been successfully written.
    ///   - Ok(false):  the message has been queued, but not yet fully written.
    ///                 will be attempted in the next write schedule.
    ///   - Err(error): there was an error while writing to the socket.
    pub fn write<T: Serialize>(&mut self, msg: Option<(T, Priority)>) -> ::Res<bool> {
        match *self {
            Socket::Udp(ref mut sock) => sock.write(msg),
            Socket::Tcp(ref mut sock) => sock.write(msg),
        }
    }

    /// Sets linger time for connection based protocols.
    pub fn set_linger(&self, dur: Option<Duration>) -> ::Res<()> {
        match *self {
            Socket::Udp(_) => Ok(()), // do nothing for UDP socket
            Socket::Tcp(ref sock) => sock.set_linger(dur),
        }
    }
}

impl From<UdpSock> for Socket {
    fn from(sock: UdpSock) -> Socket {
        Socket::Udp(sock)
    }
}

impl From<TcpSock> for Socket {
    fn from(sock: TcpSock) -> Socket {
        Socket::Tcp(sock)
    }
}
