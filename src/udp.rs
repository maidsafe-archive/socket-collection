use crypto::{DecryptContext, EncryptContext};
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

    /// Specify data encryption context which will determine how outgoing data is encrypted.
    pub fn use_encrypt_ctx(&mut self, enc_ctx: EncryptContext) -> ::Res<()> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.use_encrypt_ctx(enc_ctx);
        Ok(())
    }

    /// Specify data decryption context which will determine how incoming data is decrypted.
    pub fn use_decrypt_ctx(&mut self, dec_ctx: DecryptContext) -> ::Res<()> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.use_decrypt_ctx(dec_ctx);
        Ok(())
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
    enc_ctx: EncryptContext,
    dec_ctx: DecryptContext,
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
            out_queue2: OutQueue::new(conf.clone()),
            current_write_2: None,
            enc_ctx: conf.enc_ctx,
            dec_ctx: conf.dec_ctx,
        }
    }

    fn use_encrypt_ctx(&mut self, enc_ctx: EncryptContext) {
        self.enc_ctx = enc_ctx;
    }

    fn use_decrypt_ctx(&mut self, dec_ctx: DecryptContext) {
        self.dec_ctx = dec_ctx;
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
            return Ok(Some(self.dec_ctx.decrypt(&data)?));
        }

        // the mio reading window is max at 64k (64 * 1024)
        const BUF_LEN: usize = 64 * 1024;
        let mut buffer = [0; BUF_LEN];

        loop {
            let res = self
                .sock
                .recv(&mut buffer)
                .map(|bytes_read| buffer[..bytes_read].to_vec());
            if let RecvResult::WouldBlock(data) = handle_recv_res(res, &mut self.read_buffer)? {
                return match data {
                    Some(buf) => Ok(Some(self.dec_ctx.decrypt(&buf)?)),
                    None => Ok(None),
                };
            }
        }
    }

    fn read_frm<T: DeserializeOwned + Serialize>(&mut self) -> ::Res<Option<(T, SocketAddr)>> {
        if let Some((data, peer)) = self.read_buffer_2.pop_front() {
            return Ok(Some((self.dec_ctx.decrypt(&data)?, peer)));
        }

        // the mio reading window is max at 64k (64 * 1024)
        const BUF_LEN: usize = 64 * 1024;
        let mut buffer = [0; BUF_LEN];

        loop {
            let res = self
                .sock
                .recv_from(&mut buffer)
                .map(|(bytes_read, sender_addr)| (buffer[..bytes_read].to_vec(), sender_addr));
            if let RecvResult::WouldBlock(data) = handle_recv_res(res, &mut self.read_buffer_2)? {
                return match data {
                    Some((buf, sender_addr)) => {
                        Ok(Some((self.dec_ctx.decrypt(&buf)?, sender_addr)))
                    }
                    None => Ok(None),
                };
            }
        }
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
            self.out_queue.push(self.enc_ctx.encrypt(&msg)?, priority);
        }
        self.flush_write_until_would_block()
    }

    fn write_to<T: Serialize>(&mut self, msg: Option<(T, SocketAddr, Priority)>) -> ::Res<bool> {
        let _ = self.out_queue2.drop_expired();
        if let Some((msg, peer, priority)) = msg {
            self.out_queue2
                .push((self.enc_ctx.encrypt(&msg)?, peer), priority);
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
            if !handle_send_res(res, &mut self.current_write, data.len(), data)? {
                return Ok(false);
            }
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
            if !handle_send_res(res, &mut self.current_write_2, data.len(), (data, peer))? {
                return Ok(false);
            }
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

/// UDP socket receive result. It yields data only when socket returns EWOULDBLOCK.
enum RecvResult<T> {
    /// Stop reading socket and return some data, if any was read.
    WouldBlock(Option<T>),
    /// Keep reading UDP socket until it returns EWOULDBLOCK.
    ContinueRecv,
}

/// Buffers received UDP packets and checks when to stop calling `recv()`.
fn handle_recv_res<T: IsEmpty>(
    recv_res: io::Result<T>,
    read_buffer: &mut VecDeque<T>,
) -> ::Res<RecvResult<T>> {
    match recv_res {
        Ok(msg) => {
            if !msg.is_empty() {
                read_buffer.push_back(msg);
            }
            Ok(RecvResult::ContinueRecv)
        }
        Err(error) => {
            if error.kind() == ErrorKind::WouldBlock || error.kind() == ErrorKind::Interrupted {
                Ok(RecvResult::WouldBlock(read_buffer.pop_front()))
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
            fn it_returns_would_block() {
                let mut read_buf: VecDeque<Vec<u8>> = VecDeque::new();

                let res = handle_recv_res(Err(ErrorKind::WouldBlock.into()), &mut read_buf);

                let wouldblock = match res {
                    Ok(RecvResult::WouldBlock(_)) => true,
                    _ => false,
                };
                assert!(wouldblock);
            }

            #[test]
            fn it_returns_last_buffered_item() {
                let mut read_buf: VecDeque<Vec<u8>> = VecDeque::new();
                read_buf.push_front(vec![1, 2, 3]);

                let res = handle_recv_res(Err(ErrorKind::WouldBlock.into()), &mut read_buf);

                match res {
                    Ok(RecvResult::WouldBlock(next_msg)) => {
                        assert_eq!(next_msg, Some(vec![1, 2, 3]));
                    }
                    _ => panic!("Expected WouldBlock with data"),
                }
            }
        }

        mod when_result_is_received_buffer {
            use super::*;

            #[test]
            fn it_pushes_buffer_to_the_read_queue() {
                let mut read_buf: VecDeque<Vec<u8>> = VecDeque::new();

                let _ = handle_recv_res(Ok(vec![1, 2, 3]), &mut read_buf);

                assert_eq!(read_buf[0], vec![1, 2, 3]);
            }

            #[test]
            fn when_buff_is_empty_it_just_returns_continue_recv() {
                let mut read_buf: VecDeque<Vec<u8>> = VecDeque::new();

                let res = handle_recv_res(Ok(vec![]), &mut read_buf);

                match res {
                    Ok(RecvResult::ContinueRecv) => (),
                    _ => panic!("Expected WouldBlock with data"),
                }
                assert_eq!(read_buf.len(), 0);
            }
        }
    }
}
