// Copyright 2018 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use crate::crypto::{DecryptContext, EncryptContext};
use mio::tcp::TcpStream;
use mio::{Evented, Poll, PollOpt, Ready, Token};
use crate::out_queue::OutQueue;
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use std::fmt::{self, Debug, Formatter};
use std::io::{self, Cursor, ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr};
use std::time::Duration;
use crate::{Priority, SocketConfig, SocketError};

/// TCP socket which by default is uninitialized.
/// Asynchronous TCP socket wrapper with some specific behavior to our use cases:
///
/// * `TcpSock` takes care of message serialization and it's boundaries - no need to mess with the
///   byte stream.
/// * The maximum message length is [`DEFAULT_MAX_PAYLOAD_SIZE`].
/// * Incoming/outgoing messages are buffered.
/// * All messages are encrypted with [`EncryptContext`] and decrypted with [`DecryptContext`] that
///   can be changed at any time.
///
/// The socket is uninitialized by default and will result with [`SocketError`], if used.
///
/// [`DEFAULT_MAX_PAYLOAD_SIZE`]: constant.DEFAULT_MAX_PAYLOAD_SIZE.html
/// [`EncryptContext`]: enum.EncryptContext.html
/// [`DecryptContext`]: enum.DecryptContext.html
/// [`SocketError`]: enum.SocketError.html
pub struct TcpSock {
    inner: Option<Inner>,
}

impl TcpSock {
    /// Starts TCP connection. Returns immediately, so make sure to wait until the socket becomes
    /// writable.
    pub fn connect(addr: &SocketAddr) -> crate::Res<Self> {
        Self::connect_with_conf(addr, Default::default())
    }

    /// Starts TCP connection and uses given socket configuration. Returns immediately, so make
    /// sure to wait until the socket becomes writable.
    pub fn connect_with_conf(addr: &SocketAddr, conf: SocketConfig) -> crate::Res<Self> {
        let stream = TcpStream::connect(addr)?;
        Ok(Self::wrap_with_conf(stream, conf))
    }

    /// Wraps [`TcpStream`] and uses default socket configuration.
    ///
    /// [`TcpStream`]: https://docs.rs/mio/0.6.*/mio/net/struct.TcpStream.html
    pub fn wrap(stream: TcpStream) -> Self {
        Self {
            inner: Some(Inner::new(stream)),
        }
    }

    /// Wraps `TcpStream` and uses given socket configuration.
    pub fn wrap_with_conf(stream: TcpStream, conf: SocketConfig) -> Self {
        Self {
            inner: Some(Inner::new_with_conf(stream, conf)),
        }
    }

    /// Specify data encryption context which will determine how outgoing data is encrypted.
    pub fn set_encrypt_ctx(&mut self, enc_ctx: EncryptContext) -> crate::Res<()> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.set_encrypt_ctx(enc_ctx);
        Ok(())
    }

    /// Specify data decryption context which will determine how incoming data is decrypted.
    pub fn set_decrypt_ctx(&mut self, dec_ctx: DecryptContext) -> crate::Res<()> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.set_decrypt_ctx(dec_ctx);
        Ok(())
    }

    /// Sets TCP [`SO_LINGER`] value for the underlying socket.
    ///
    /// [`SO_LINGER`]: https://linux.die.net/man/3/setsockopt
    pub fn set_linger(&self, dur: Option<Duration>) -> crate::Res<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.stream.set_linger(dur)?;
        Ok(())
    }

    /// Set Time To Live value for the underlying TCP socket.
    pub fn set_ttl(&self, ttl: u32) -> crate::Res<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.stream.set_ttl(ttl)?;
        Ok(())
    }

    /// Retrieve Time To Live value.
    pub fn ttl(&self) -> crate::Res<u32> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.stream.ttl()?)
    }

    /// Returns local socket address.
    pub fn local_addr(&self) -> crate::Res<SocketAddr> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.stream.local_addr()?)
    }

    /// Returns the address of the remote peer socket is connected to.
    pub fn peer_addr(&self) -> crate::Res<SocketAddr> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.stream.peer_addr()?)
    }

    /// Retrieve last socket error, if one exists.
    pub fn take_error(&self) -> crate::Res<Option<io::Error>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.stream.take_error()?)
    }

    /// Read message from the socket. Call this from inside the `ready` handler.
    ///
    /// # Returns:
    ///
    ///   - Ok(Some(data)): data has been successfully read from the socket
    ///   - Ok(None):       there is not enough data in the socket. Call `read()`
    ///                     again in the next invocation of the `ready` handler.
    ///   - Err(error):     there was an error reading from the socket.
    pub fn read<T: Serialize + DeserializeOwned>(&mut self) -> crate::Res<Option<T>> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.read()
    }

    /// Write a message to the socket.
    ///
    /// # Returns:
    ///
    ///   - Ok(true):   the message has been successfully written.
    ///   - Ok(false):  the message has been queued, but not yet fully written.
    ///                 will be attempted in the next write schedule.
    ///   - Err(error): there was an error while writing to the socket.
    pub fn write<T: Serialize>(&mut self, msg: Option<(T, Priority)>) -> crate::Res<bool> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.write(msg)
    }
}

impl Debug for TcpSock {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "TcpSock: initialised = {}", self.inner.is_some())
    }
}

impl Default for TcpSock {
    fn default() -> Self {
        TcpSock { inner: None }
    }
}

impl Evented for TcpSock {
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
    stream: TcpStream,
    msg_reader: LenDelimitedReader,
    out_queue: OutQueue<Vec<u8>>,
    current_write: Option<Vec<u8>>,
    enc_ctx: EncryptContext,
}

impl Inner {
    fn new(stream: TcpStream) -> Self {
        Self::new_with_conf(stream, Default::default())
    }

    fn new_with_conf(stream: TcpStream, conf: SocketConfig) -> Self {
        let mut msg_reader = LenDelimitedReader::new(conf.max_payload_size);
        msg_reader.dec_ctx = conf.dec_ctx.clone();
        Self {
            stream,
            msg_reader,
            enc_ctx: conf.enc_ctx.clone(),
            out_queue: OutQueue::new(conf),
            current_write: None,
        }
    }

    fn set_encrypt_ctx(&mut self, enc_ctx: EncryptContext) {
        self.enc_ctx = enc_ctx;
    }

    fn set_decrypt_ctx(&mut self, dec_ctx: DecryptContext) {
        self.msg_reader.dec_ctx = dec_ctx;
    }

    fn read<T: Serialize + DeserializeOwned>(&mut self) -> crate::Res<Option<T>> {
        if let Some(message) = self.msg_reader.try_read()? {
            return Ok(Some(message));
        }

        // the mio reading window is max at 64k (64 * 1024)
        let mut buffer = [0; 64 * 1024];
        let mut is_something_read = false;

        loop {
            match self.stream.read(&mut buffer) {
                Ok(bytes_rxd) => {
                    if bytes_rxd == 0 {
                        let e = Err(SocketError::ZeroByteRead);
                        if is_something_read {
                            return match self.msg_reader.try_read() {
                                r @ Ok(Some(_)) | r @ Err(_) => r,
                                Ok(None) => e,
                            };
                        } else {
                            return e;
                        }
                    }
                    self.msg_reader.put_buf(&buffer[..bytes_rxd]);
                    is_something_read = true;
                }
                Err(error) => {
                    return if error.kind() == ErrorKind::WouldBlock
                        || error.kind() == ErrorKind::Interrupted
                    {
                        if is_something_read {
                            self.msg_reader.try_read()
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

    fn write<T: Serialize>(&mut self, msg: Option<(T, Priority)>) -> crate::Res<bool> {
        let _ = self.out_queue.drop_expired();
        if let Some((msg, priority)) = msg {
            self.enqueue_data(priority, msg)?;
        }
        self.flush_write_until_would_block()
    }

    /// Returns `Ok(false)`, if write to the underlying stream would block.
    fn flush_write_until_would_block(&mut self) -> crate::Res<bool> {
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

            match self.stream.write(&data) {
                Ok(bytes_txd) => {
                    if bytes_txd < data.len() {
                        self.current_write = Some(data[bytes_txd..].to_owned());
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

    fn enqueue_data<T: Serialize>(&mut self, priority: Priority, msg: T) -> crate::Res<()> {
        let buf = serialize_with_len(msg, &self.enc_ctx)?;
        self.out_queue.push(buf, priority);
        Ok(())
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
        self.stream.register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        self.stream.reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        self.stream.deregister(poll)
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

/// Serialize given value and write to a buffer with 4 byte length header.
fn serialize_with_len<T: Serialize>(value: T, enc_ctx: &EncryptContext) -> crate::Res<Vec<u8>> {
    let encrypted_data = enc_ctx.encrypt(&value)?;
    let encrypted_len = enc_ctx.encrypt(&(encrypted_data.len() as u32))?;

    let mut data = Cursor::new(Vec::with_capacity(
        encrypted_len.len() + encrypted_data.len(),
    ));
    // TODO(povilas): if safe_crypto implements encrypt_into, use that to reduce data copying
    let _ = data.write(&encrypted_len)?;
    let _ = data.write(&encrypted_data)?;

    Ok(data.into_inner())
}

/// Reads length delimited data frames.
struct LenDelimitedReader {
    read_buffer: Vec<u8>,
    read_len: usize,
    max_payload_size: usize,
    dec_ctx: DecryptContext,
}

impl LenDelimitedReader {
    /// Construct reader with empty read buffer.
    fn new(max_payload_size: usize) -> Self {
        Self {
            read_buffer: Vec::new(),
            read_len: 0,
            max_payload_size,
            dec_ctx: Default::default(),
        }
    }

    /// Puts data buffer into the reader so that later when enough data is put it could be read and
    /// deserialised.
    fn put_buf(&mut self, buf: &[u8]) {
        self.read_buffer.extend_from_slice(buf);
    }

    fn try_read<T: Serialize + DeserializeOwned>(&mut self) -> crate::Res<Option<T>> {
        if self.read_len == 0 && !self.try_read_header()? {
            return Ok(None);
        }
        if self.read_len > self.read_buffer.len() {
            return Ok(None);
        }

        let result = self.dec_ctx.decrypt(&self.read_buffer[..self.read_len])?;
        self.mark_read();
        Ok(Some(result))
    }

    fn try_read_header(&mut self) -> crate::Res<bool> {
        if self.read_buffer.len() < self.dec_ctx.encrypted_size_len() {
            return Ok(false);
        }

        let data_len: u32 = self
            .dec_ctx
            .decrypt(&self.read_buffer[..self.dec_ctx.encrypted_size_len()])?;
        self.read_len = data_len as usize;

        if self.read_len > self.max_payload_size {
            return Err(SocketError::PayloadSizeProhibitive);
        }

        self.read_buffer = self.read_buffer[self.dec_ctx.encrypted_size_len()..].to_owned();
        Ok(self.read_len > 0)
    }

    /// Reset read buffer to the beginning of the next message.
    fn mark_read(&mut self) {
        self.read_buffer = self.read_buffer[self.read_len..].to_owned();
        self.read_len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hamcrest2::prelude::*;
    use maidsafe_utilities::serialisation::serialise;
    use safe_crypto::gen_encrypt_keypair;
    use crate::DEFAULT_MAX_PAYLOAD_SIZE;

    mod serialize_with_len {
        use super::*;

        proptest! {
            #[test]
            fn it_writes_encrypted_data_length(data_len in (0..65000)) {
                let data_len = data_len as usize;
                let exp_serialised = unwrap!(serialise(&vec![1u8; data_len]));
                let crypto_ctx = EncryptContext::null();

                let buf = unwrap!(serialize_with_len(vec![1u8; data_len], &crypto_ctx));

                assert_that!(buf.len(), eq(exp_serialised.len() + crypto_ctx.encrypted_size_len()));
            }
        }
    }

    mod len_delimited_reader {
        use super::*;

        mod try_read {
            use super::*;

            #[test]
            fn it_deserializes_data_from_bytes() {
                let mut reader = LenDelimitedReader::new(2 * 1024 * 1024);
                let buf = unwrap!(serialize_with_len(vec![1, 2, 3], &EncryptContext::null()));
                reader.put_buf(&buf);

                let data = unwrap!(reader.try_read());

                assert_eq!(data, Some(vec![1, 2, 3]));
            }

            #[test]
            fn it_deserializes_data_from_bytes_when_there_is_extra_bytes_buffered() {
                let mut reader = LenDelimitedReader::new(2 * 1024 * 1024);
                let buf = unwrap!(serialize_with_len(vec![1, 2, 3], &EncryptContext::null()));
                reader.put_buf(&buf);
                let buf = unwrap!(serialize_with_len(vec![4], &EncryptContext::null()));
                reader.put_buf(&buf);

                let data = unwrap!(reader.try_read());

                assert_eq!(data, Some(vec![1, 2, 3]));
            }

            #[test]
            fn when_data_len_is_0_it_returns_none() {
                let mut reader = LenDelimitedReader::new(2 * 1024 * 1024);
                reader.put_buf(&[0, 0, 0, 0]);

                let data: Option<Vec<u8>> = unwrap!(reader.try_read());

                assert_eq!(data, None);
            }
        }

        mod try_read_header {
            use super::*;

            #[test]
            fn when_data_len_is_0_it_returns_false() {
                let mut reader = LenDelimitedReader::new(2 * 1024 * 1024);
                reader.put_buf(&[0, 0, 0, 0]);

                let res = unwrap!(reader.try_read_header());

                assert_eq!(res, false);
                assert_eq!(reader.read_len, 0);
            }
        }
    }

    #[test]
    fn data_read_write_with_encryption() {
        let (pk1, sk1) = gen_encrypt_keypair();
        let (pk2, sk2) = gen_encrypt_keypair();

        let enc_key1 = sk1.shared_secret(&pk2);
        let enc_key2 = sk2.shared_secret(&pk1);

        let serialised = unwrap!(serialize_with_len(
            "message123".to_owned(),
            &EncryptContext::authenticated(enc_key1)
        ));

        let mut reader = LenDelimitedReader::new(DEFAULT_MAX_PAYLOAD_SIZE);
        reader.dec_ctx = DecryptContext::authenticated(enc_key2);
        reader.put_buf(&serialised);
        let deserialised: Option<String> = unwrap!(reader.try_read());

        assert_eq!(deserialised, Some("message123".to_owned()));
    }
}
