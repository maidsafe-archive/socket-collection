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
        if let Some(data) = self.read_buffer.pop_front() {
            return Ok(Some(deserialise(&data)?));
        }

        // the mio reading window is max at 64k (64 * 1024)
        const BUF_LEN: usize = 64 * 1024;
        let mut buffer = [0; BUF_LEN];
        let mut would_block = false;

        while !would_block {
            let res = self
                .sock
                .recv(&mut buffer)
                .map(|bytes_read| buffer[..bytes_read].to_vec());
            if let Some(buf) = handle_recv_res(res, &mut self.read_buffer, &mut would_block)? {
                return Ok(Some(deserialise(&buf)?));
            }
        }
        Ok(None)
    }

    fn read_frm<T: DeserializeOwned + Serialize>(&mut self) -> ::Res<Option<(T, SocketAddr)>> {
        if let Some((data, peer)) = self.read_buffer_2.pop_front() {
            // The max buffer size we have is 64 * 1024 bytes so calling deserialise instead of
            // deserialise_with_limit() which will require us to use `Bounded` which is a type in
            // bincode. FIXME: get maidsafe-utils to take a size instead of `Bounded` type
            return Ok(Some((deserialise(&data)?, peer)));
        }

        // the mio reading window is max at 64k (64 * 1024)
        const BUF_LEN: usize = 64 * 1024;
        let mut buffer = [0; BUF_LEN];
        let mut would_block = false;

        while !would_block {
            let res = self
                .sock
                .recv_from(&mut buffer)
                .map(|(bytes_read, sender_addr)| (buffer[..bytes_read].to_vec(), sender_addr));
            if let Some((buf, sender_addr)) =
                handle_recv_res(res, &mut self.read_buffer_2, &mut would_block)?
            {
                return Ok(Some((deserialise(&buf)?, sender_addr)));
            }
        }
        Ok(None)
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
        self.flush_write_until_would_block()
    }

    fn write_to<T: Serialize>(&mut self, msg: Option<(T, SocketAddr, Priority)>) -> ::Res<bool> {
        let _ = self.out_queue2.drop_expired();
        if let Some((msg, peer, priority)) = msg {
            self.out_queue2.push((serialise(&msg)?, peer), priority);
        }
        self.flush_write_to_until_would_block()
    }

    /// Returns `Ok(false)`, if write to the underlying stream would block.
    fn flush_write_until_would_block(&mut self) -> ::Res<bool> {
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

            let res = self.sock.send(&data);
            handle_send_res(res, &mut self.current_write, data.len(), data)?;
        }
    }

    /// Returns `Ok(false)`, if write to the underlying stream would block.
    fn flush_write_to_until_would_block(&mut self) -> ::Res<bool> {
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

            let res = self.sock.send_to(&data, &peer);
            handle_send_res(res, &mut self.current_write_2, data.len(), (data, peer))?;
        }
    }
}

/// Helper trait for `handle_recv_res()`.
trait IsEmpty {
    fn is_empty(&self) -> bool;
}

impl IsEmpty for Vec<u8> {
    fn is_empty(&self) -> bool {
        self.is_empty()
    }
}

impl IsEmpty for (Vec<u8>, SocketAddr) {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn handle_recv_res<T: IsEmpty>(
    recv_res: io::Result<T>,
    read_buffer: &mut VecDeque<T>,
    would_block: &mut bool,
) -> ::Res<Option<T>> {
    match recv_res {
        Ok(msg) => {
            if msg.is_empty() {
                let e = Err(SocketError::ZeroByteRead);
                if !read_buffer.is_empty() {
                    return match read_buffer.pop_front() {
                        Some(msg) => Ok(Some(msg)),
                        None => e,
                    };
                } else {
                    return e;
                }
            }
            read_buffer.push_back(msg);
            Ok(None)
        }
        Err(error) => {
            return if error.kind() == ErrorKind::WouldBlock
                || error.kind() == ErrorKind::Interrupted
            {
                *would_block = true;
                Ok(read_buffer.pop_front())
            } else {
                Err(From::from(error))
            }
        }
    }
}

/// If failed to send a messages, requeue it.
fn handle_send_res<M>(
    send_res: io::Result<usize>,
    current_write: &mut Option<M>,
    msg_len: usize,
    msg: M,
) -> ::Res<bool> {
    match send_res {
        Ok(bytes_txd) => {
            if bytes_txd < msg_len {
                warn!(
                    "Partial datagram sent. Will likely be interpreted as corrupted.
                       Queued to be sent again."
                );
                *current_write = Some(msg);
            }
            Ok(true)
        }
        Err(error) => {
            if error.kind() == ErrorKind::WouldBlock || error.kind() == ErrorKind::Interrupted {
                *current_write = Some(msg);
                Ok(false)
            } else {
                Err(From::from(error))
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

#[cfg(test)]
mod tests {
    use super::*;

    mod handle_send_res {
        use super::*;

        #[test]
        fn it_doesnt_alter_current_write_when_all_bytes_were_sent() {
            let mut current_write = None;

            let res = handle_send_res(Ok(100), &mut current_write, 100, vec![1; 100]);

            assert_eq!(unwrap!(res), true);
            assert_eq!(current_write, None);
        }

        #[test]
        fn when_partial_message_was_sent_it_sets_current_write_to_data_to_be_sent_again() {
            let mut current_write = None;

            let res = handle_send_res(Ok(3), &mut current_write, 5, vec![1, 2, 3, 4, 5]);

            assert_eq!(unwrap!(res), true);
            assert_eq!(current_write, Some(vec![1, 2, 3, 4, 5]));
        }

        #[test]
        fn when_result_is_error_would_block_it_returns_false() {
            let mut current_write = None;

            let res = handle_send_res(
                Err(ErrorKind::WouldBlock.into()),
                &mut current_write,
                5,
                vec![1, 2, 3, 4, 5],
            );

            assert_eq!(unwrap!(res), false);
        }

        #[test]
        fn when_result_is_error_would_block_it_sets_current_write_to_data_to_be_sent_again() {
            let mut current_write = None;

            let _ = handle_send_res(
                Err(ErrorKind::WouldBlock.into()),
                &mut current_write,
                5,
                vec![1, 2, 3, 4, 5],
            );

            assert_eq!(current_write, Some(vec![1, 2, 3, 4, 5]));
        }
    }

    mod handle_recv_res {
        use super::*;

        mod when_result_is_error_would_block {
            use super::*;

            #[test]
            fn it_sets_would_block_to_true() {
                let mut read_buf: VecDeque<Vec<u8>> = VecDeque::new();
                let mut would_block = false;

                let res = handle_recv_res(
                    Err(ErrorKind::WouldBlock.into()),
                    &mut read_buf,
                    &mut would_block,
                );

                assert!(would_block);
                assert_eq!(unwrap!(res), None);
            }

            #[test]
            fn it_returns_last_buffered_item() {
                let mut read_buf: VecDeque<Vec<u8>> = VecDeque::new();
                read_buf.push_front(vec![1, 2, 3]);
                let mut would_block = false;

                let next_msg = handle_recv_res(
                    Err(ErrorKind::WouldBlock.into()),
                    &mut read_buf,
                    &mut would_block,
                );

                assert_eq!(unwrap!(next_msg), Some(vec![1, 2, 3]));
            }
        }

        mod when_result_is_received_buffer {
            use super::*;

            #[test]
            fn it_pushes_buffer_to_the_read_queue() {
                let mut read_buf: VecDeque<Vec<u8>> = VecDeque::new();
                let mut would_block = false;

                let next_msg = handle_recv_res(Ok(vec![1, 2, 3]), &mut read_buf, &mut would_block);

                assert_eq!(unwrap!(next_msg), None);
                assert_eq!(read_buf[0], vec![1, 2, 3]);
            }
        }
    }
}
