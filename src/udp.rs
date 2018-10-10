use maidsafe_utilities::serialisation::{deserialise, serialise};
use mio::net::UdpSocket;
use mio::{Evented, Poll, PollOpt, Ready, Token};
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::io::{self, ErrorKind};
use std::net::SocketAddr;
use std::time::Instant;
use {Priority, SocketError, MAX_MSG_AGE_SECS, MSG_DROP_PRIORITY};

pub struct UdpSock {
    inner: Option<Inner>,
}

impl UdpSock {
    pub fn wrap(sock: UdpSocket) -> Self {
        Self {
            inner: Some(Inner {
                sock,
                peer: None,
                read_buffer: Default::default(),
                read_len: 0,
                write_queue: Default::default(),
                current_write: None,
            }),
        }
    }

    pub fn bind(&self, addr: &SocketAddr) -> ::Res<Self> {
        Ok(Self::wrap(UdpSocket::bind(addr)?))
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

    pub fn set_ttl(&self, ttl: u32) -> ::Res<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.sock.set_ttl(ttl)?)
    }

    pub fn ttl(&self) -> ::Res<u32> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.sock.ttl()?)
    }

    pub fn peer_addr(&self) -> ::Res<SocketAddr> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.peer.ok_or(SocketError::UnconnectedUdpSocket)?)
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
    read_len: usize,
    write_queue: BTreeMap<Priority, VecDeque<(Instant, Vec<u8>)>>,
    current_write: Option<Vec<u8>>,
}

impl Inner {
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
                        if is_something_read {
                            self.read_from_buffer()
                        } else {
                            Ok(None)
                        }
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

    // Write a message to the socket.
    //
    // Returns:
    //   - Ok(true):   the message has been successfully written.
    //   - Ok(false):  the message has been queued, but not yet fully written.
    //                 will be attempted in the next write schedule.
    //   - Err(error): there was an error while writing to the socket.
    fn write<T: Serialize>(&mut self, msg: Option<(T, Priority)>) -> ::Res<bool> {
        let expired_keys: Vec<u8> = self
            .write_queue
            .iter()
            .skip_while(|&(&priority, queue)| {
                priority < MSG_DROP_PRIORITY || // Don't drop high-priority messages.
                queue.front().map_or(true, |&(ref timestamp, _)| {
                    timestamp.elapsed().as_secs() <= MAX_MSG_AGE_SECS
                })
            }).map(|(&priority, _)| priority)
            .collect();
        let dropped_msgs: usize = expired_keys
            .iter()
            .filter_map(|priority| self.write_queue.remove(priority))
            .map(|queue| queue.len())
            .sum();
        if dropped_msgs > 0 {
            trace!(
                "Insufficient bandwidth. Dropping {} messages with priority >= {}.",
                dropped_msgs,
                expired_keys[0]
            );
        }

        if let Some((msg, priority)) = msg {
            let entry = self
                .write_queue
                .entry(priority)
                .or_insert_with(|| VecDeque::with_capacity(10));
            entry.push_back((Instant::now(), serialise(&msg)?));
        }

        loop {
            let data = if let Some(data) = self.current_write.take() {
                data
            } else {
                let (key, (_time_stamp, data), empty) = match self.write_queue.iter_mut().next() {
                    Some((key, queue)) => (*key, unwrap!(queue.pop_front()), queue.is_empty()),
                    None => return Ok(true),
                };
                if empty {
                    let _ = self.write_queue.remove(&key);
                }
                data
            };

            match self.sock.send(&data) {
                Ok(bytes_txd) => {
                    if bytes_txd < data.len() {
                        debug!(
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
