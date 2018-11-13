use maidsafe_utilities::serialisation::{deserialise, serialise};
use mio::net::UdpSocket;
use mio::{Evented, Poll, PollOpt, Ready, Token};
use out_queue::OutQueue;
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use std::collections::VecDeque;
use std::fmt::{self, Debug, Formatter};
use std::io::{self, ErrorKind};
use std::net::SocketAddr;
use {Priority, SocketConfig, SocketError};

pub struct UdpSock {
    inner: Option<Inner>,
}

impl UdpSock {
    /// Wrap `UdpSocket` and use default socket configuration.
    pub fn wrap(sock: UdpSocket) -> Self {
        Self::wrap_with_conf(sock, Default::default())
    }

    /// Wrap `UdpSocket` and use given socket configuration.
    pub fn wrap_with_conf(sock: UdpSocket, conf: SocketConfig) -> Self {
        Self {
            inner: Some(Inner::new(sock, conf)),
        }
    }

    /// Create new `UdpSock` bound to the given address with default configuration.
    pub fn bind(addr: &SocketAddr) -> ::Res<Self> {
        Self::bind_with_conf(addr, Default::default())
    }

    /// Create new `UdpSock` bound to the given address with given configuration.
    pub fn bind_with_conf(addr: &SocketAddr, conf: SocketConfig) -> ::Res<Self> {
        Ok(Self::wrap_with_conf(UdpSocket::bind(addr)?, conf))
    }

    pub fn connect(&mut self, addr: &SocketAddr) -> ::Res<()> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.sock.connect(*addr)?;
        inner.peer = Some(*addr);

        Ok(())
    }

    pub fn local_addr(&self) -> ::Res<SocketAddr> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.sock.local_addr()?)
    }

    pub fn peer_addr(&self) -> ::Res<SocketAddr> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.peer.ok_or(SocketError::UnconnectedUdpSocket)?)
    }

    pub fn set_ttl(&self, ttl: u32) -> ::Res<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.sock.set_ttl(ttl)?;
        Ok(())
    }

    pub fn ttl(&self) -> ::Res<u32> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.sock.ttl()?)
    }

    pub fn take_error(&self) -> ::Res<Option<io::Error>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.sock.take_error()?)
    }

    // Read message from the socket. Call this from inside the `ready` handler.
    //
    // Returns:
    //   - Ok(Some(data)): data has been successfully read from the socket
    //   - Ok(None):       there is not enough data in the socket. Call `read`
    //                     again in the next invocation of the `ready` handler.
    //   - Err(error):     there was an error reading from the socket.
    pub fn read<T: DeserializeOwned + Serialize>(&mut self) -> ::Res<Option<T>> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.read()
    }

    pub fn read_frm<T: DeserializeOwned + Serialize>(&mut self) -> ::Res<Option<(T, SocketAddr)>> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.read_frm()
    }

    // Write a message to the socket.
    //
    // Returns:
    //   - Ok(true):   the message has been successfully written.
    //   - Ok(false):  the message has been queued, but not yet fully written.
    //                 will be attempted in the next write schedule.
    //   - Err(error): there was an error while writing to the socket.
    pub fn write<T: Serialize>(&mut self, msg: Option<(T, Priority)>) -> ::Res<bool> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.write(msg)
    }

    pub fn write_to<T: Serialize>(
        &mut self,
        msg: Option<(T, SocketAddr, Priority)>,
    ) -> ::Res<bool> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.write_to(msg)
    }

    pub fn into_underlying_sock(mut self) -> ::Res<UdpSocket> {
        let inner = self.inner.take().ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.sock)
    }
}

impl Debug for UdpSock {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "UdpSock: initialised = {}", self.inner.is_some())
    }
}

impl Default for UdpSock {
    fn default() -> Self {
        UdpSock { inner: None }
    }
}

impl Evented for UdpSock {
    fn register(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        let inner = self.inner.as_ref().ok_or_else(|| {
            io::Error::new(
                ErrorKind::Other,
                format!("{}", SocketError::UninitialisedSocket),
            )
        })?;
        inner.register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        let inner = self.inner.as_ref().ok_or_else(|| {
            io::Error::new(
                ErrorKind::Other,
                format!("{}", SocketError::UninitialisedSocket),
            )
        })?;
        inner.reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        let inner = self.inner.as_ref().ok_or_else(|| {
            io::Error::new(
                ErrorKind::Other,
                format!("{}", SocketError::UninitialisedSocket),
            )
        })?;
        inner.deregister(poll)
    }
}

struct Inner {
    sock: UdpSocket,
    peer: Option<SocketAddr>,
    read_buffer: VecDeque<Vec<u8>>,
    read_buffer_2: VecDeque<(Vec<u8>, SocketAddr)>,
    out_queue: OutQueue<Vec<u8>>,
    current_write: Option<Vec<u8>>,
    out_queue2: OutQueue<(Vec<u8>, SocketAddr)>,
    current_write_2: Option<(Vec<u8>, SocketAddr)>,
}

impl Inner {
    fn new(sock: UdpSocket, conf: SocketConfig) -> Self {
        Self {
            sock,
            peer: None,
            read_buffer: Default::default(),
            read_buffer_2: Default::default(),
            out_queue: OutQueue::new(conf.clone()),
            current_write: None,
            out_queue2: OutQueue::new(conf),
            current_write_2: None,
        }
    }

    // Read message from the socket. Call this from inside the `ready` handler.
    //
    // Returns:
    //   - Ok(Some(data)): data has been successfully read from the socket.
    //   - Ok(None):       there is not enough data in the socket. Call `read`
    //                     again in the next invocation of the `ready` handler.
    //   - Err(error):     there was an error reading from the socket.
    fn read<T: DeserializeOwned + Serialize>(&mut self) -> ::Res<Option<T>> {
        if let Some(message) = self.read_from_buffer()? {
            return Ok(Some(message));
        }

        // the mio reading window is max at 64k (64 * 1024)
        const BUF_LEN: usize = 64 * 1024;
        let mut buffer = [0; BUF_LEN];
        let mut is_something_read = false;

        loop {
            match self.sock.recv(&mut buffer) {
                Ok(bytes_rxd) => {
                    if bytes_rxd == 0 {
                        let e = Err(SocketError::ZeroByteRead);
                        if is_something_read {
                            return match self.read_from_buffer() {
                                r @ Ok(Some(_)) | r @ Err(_) => r,
                                Ok(None) => e,
                            };
                        } else {
                            return e;
                        }
                    }
                    self.read_buffer.push_back(buffer[..bytes_rxd].to_vec());
                    is_something_read = true;
                }
                Err(error) => {
                    return if error.kind() == ErrorKind::WouldBlock
                        || error.kind() == ErrorKind::Interrupted
                    {
                        self.read_from_buffer()
                    } else {
                        Err(From::from(error))
                    }
                }
            }
        }
    }

    fn read_from_buffer<T: DeserializeOwned + Serialize>(&mut self) -> ::Res<Option<T>> {
        let data = match self.read_buffer.pop_front() {
            Some(d) => d,
            None => return Ok(None),
        };

        // The max buffer size we have is 64 * 1024 bytes so calling deserialise instead of
        // deserialise_with_limit() which will require us to use `Bounded` which is a type in
        // bincode. FIXME: get maidsafe-utils to take a size instead of `Bounded` type
        Ok(Some(deserialise(&data)?))
    }

    fn read_frm<T: DeserializeOwned + Serialize>(&mut self) -> ::Res<Option<(T, SocketAddr)>> {
        if let Some(pkg) = self.read_from_buffer_2()? {
            return Ok(Some(pkg));
        }

        // the mio reading window is max at 64k (64 * 1024)
        const BUF_LEN: usize = 64 * 1024;
        let mut buffer = [0; BUF_LEN];
        let mut is_something_read = false;

        loop {
            match self.sock.recv_from(&mut buffer) {
                Ok((bytes_rxd, peer)) => {
                    if bytes_rxd == 0 {
                        let e = Err(SocketError::ZeroByteRead);
                        if is_something_read {
                            return match self.read_from_buffer_2() {
                                r @ Ok(Some(_)) | r @ Err(_) => r,
                                Ok(None) => e,
                            };
                        } else {
                            return e;
                        }
                    }
                    self.read_buffer_2
                        .push_back((buffer[..bytes_rxd].to_vec(), peer));
                    is_something_read = true;
                }
                Err(error) => {
                    return if error.kind() == ErrorKind::WouldBlock
                        || error.kind() == ErrorKind::Interrupted
                    {
                        self.read_from_buffer_2()
                    } else {
                        Err(From::from(error))
                    }
                }
            }
        }
    }

    fn read_from_buffer_2<T: DeserializeOwned + Serialize>(
        &mut self,
    ) -> ::Res<Option<(T, SocketAddr)>> {
        let (data, peer) = match self.read_buffer_2.pop_front() {
            Some(pkg) => pkg,
            None => return Ok(None),
        };

        // The max buffer size we have is 64 * 1024 bytes so calling deserialise instead of
        // deserialise_with_limit() which will require us to use `Bounded` which is a type in
        // bincode. FIXME: get maidsafe-utils to take a size instead of `Bounded` type
        Ok(Some((deserialise(&data)?, peer)))
    }

    // Write a message to the socket.
    //
    // Returns:
    //   - Ok(true):   the message has been successfully written.
    //   - Ok(false):  the message has been queued, but not yet fully written.
    //                 will be attempted in the next write schedule.
    //   - Err(error): there was an error while writing to the socket.
    fn write<T: Serialize>(&mut self, msg: Option<(T, Priority)>) -> ::Res<bool> {
        let _ = self.out_queue.drop_expired();
        if let Some((msg, priority)) = msg {
            self.out_queue.push(serialise(&msg)?, priority);
        }

        loop {
            let data = if let Some(data) = self
                .current_write
                .take()
                .or_else(|| self.out_queue.next_msg())
            {
                data
            } else {
                return Ok(true);
            };

            match self.sock.send(&data) {
                Ok(bytes_txd) => {
                    if bytes_txd < data.len() {
                        warn!(
                            "Partial datagram sent. Will likely be interpreted as corrupted.
                               Queued to be sent again."
                        );
                        self.current_write = Some(data);
                    }
                }
                Err(error) => {
                    if error.kind() == ErrorKind::WouldBlock
                        || error.kind() == ErrorKind::Interrupted
                    {
                        self.current_write = Some(data);
                        return Ok(false);
                    } else {
                        return Err(From::from(error));
                    }
                }
            }
        }
    }

    fn write_to<T: Serialize>(&mut self, msg: Option<(T, SocketAddr, Priority)>) -> ::Res<bool> {
        let _ = self.out_queue2.drop_expired();
        if let Some((msg, peer, priority)) = msg {
            self.out_queue2.push((serialise(&msg)?, peer), priority);
        }

        loop {
            let (data, peer) = if let Some(data_peer) = self
                .current_write_2
                .take()
                .or_else(|| self.out_queue2.next_msg())
            {
                data_peer
            } else {
                return Ok(true);
            };

            match self.sock.send_to(&data, &peer) {
                Ok(bytes_txd) => {
                    if bytes_txd < data.len() {
                        warn!(
                            "Partial datagram sent. Will likely be interpreted as corrupted.
                               Queued to be sent again."
                        );
                        self.current_write_2 = Some((data, peer));
                    }
                }
                Err(error) => {
                    if error.kind() == ErrorKind::WouldBlock
                        || error.kind() == ErrorKind::Interrupted
                    {
                        self.current_write_2 = Some((data, peer));
                        return Ok(false);
                    } else {
                        return Err(From::from(error));
                    }
                }
            }
        }
    }
}

impl Evented for Inner {
    fn register(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        self.sock.register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        self.sock.reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        self.sock.deregister(poll)
    }
}
