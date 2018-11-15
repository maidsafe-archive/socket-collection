use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use maidsafe_utilities::serialisation::{deserialise_from, serialise_into};
use mio::tcp::TcpStream;
use mio::{Evented, Poll, PollOpt, Ready, Token};
use out_queue::OutQueue;
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use std::fmt::{self, Debug, Formatter};
use std::io::{self, Cursor, ErrorKind, Read, Write};
use std::mem;
use std::net::{Shutdown, SocketAddr};
use std::time::Duration;
use {Priority, SocketConfig, SocketError};

/// TCP socket which by default is uninitialized.
pub struct TcpSock {
    inner: Option<Inner>,
}

impl TcpSock {
    /// Starts TCP connection. Returns immediately, so make sure to wait until the socket becomes
    /// writable.
    pub fn connect(addr: &SocketAddr) -> ::Res<Self> {
        Self::connect_with_conf(addr, Default::default())
    }

    /// Starts TCP connection and uses given socket configuration. Returns immediately, so make
    /// sure to wait until the socket becomes writable.
    pub fn connect_with_conf(addr: &SocketAddr, conf: SocketConfig) -> ::Res<Self> {
        let stream = TcpStream::connect(addr)?;
        Ok(Self::wrap_with_conf(stream, conf))
    }

    /// Wraps `TcpStream` and uses default socket configuration.
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

    pub fn set_linger(&self, dur: Option<Duration>) -> ::Res<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.stream.set_linger(dur)?;
        Ok(())
    }

    pub fn local_addr(&self) -> ::Res<SocketAddr> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.stream.local_addr()?)
    }

    pub fn peer_addr(&self) -> ::Res<SocketAddr> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.stream.peer_addr()?)
    }

    pub fn take_error(&self) -> ::Res<Option<io::Error>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.stream.take_error()?)
    }

    // Read message from the socket. Call this from inside the `ready` handler.
    //
    // Returns:
    //   - Ok(Some(data)): data has been successfully read from the socket
    //   - Ok(None):       there is not enough data in the socket. Call `read`
    //                     again in the next invocation of the `ready` handler.
    //   - Err(error):     there was an error reading from the socket.
    pub fn read<T: DeserializeOwned>(&mut self) -> ::Res<Option<T>> {
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
    out_queue: OutQueue,
    current_write: Option<Vec<u8>>,
}

impl Inner {
    fn new(stream: TcpStream) -> Self {
        Self::new_with_conf(stream, Default::default())
    }

    fn new_with_conf(stream: TcpStream, conf: SocketConfig) -> Self {
        Self {
            stream,
            msg_reader: LenDelimitedReader::new(conf.max_payload_size),
            out_queue: OutQueue::new(conf),
            current_write: None,
        }
    }

    // Read message from the socket. Call this from inside the `ready` handler.
    //
    // Returns:
    //   - Ok(Some(data)): data has been successfully read from the socket.
    //   - Ok(None):       there is not enough data in the socket. Call `read`
    //                     again in the next invocation of the `ready` handler.
    //   - Err(error):     there was an error reading from the socket.
    fn read<T: DeserializeOwned>(&mut self) -> ::Res<Option<T>> {
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
            self.enqueue_data(msg, priority)?;
        }
        self.flush_write_until_would_block()
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

    fn enqueue_data<T: Serialize>(&mut self, msg: T, priority: Priority) -> ::Res<()> {
        let buf = write_len_delimited(msg)?;
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
fn write_len_delimited<T: Serialize>(value: T) -> ::Res<Vec<u8>> {
    let mut data = Cursor::new(Vec::with_capacity(mem::size_of::<u32>()));

    let _ = data.write_u32::<LittleEndian>(0);
    serialise_into(&value, &mut data)?;
    let len = data.position() - mem::size_of::<u32>() as u64;
    data.set_position(0);
    data.write_u32::<LittleEndian>(len as u32)?;

    Ok(data.into_inner())
}

/// Reads length delimited data frames.
struct LenDelimitedReader {
    read_buffer: Vec<u8>,
    read_len: usize,
    max_payload_size: usize,
}

impl LenDelimitedReader {
    /// Construct reader with empty read buffer.
    fn new(max_payload_size: usize) -> Self {
        Self {
            read_buffer: Vec::new(),
            read_len: 0,
            max_payload_size,
        }
    }

    /// Puts data buffer into the reader so that later when enough data is put it could be read and
    /// deserialised.
    fn put_buf(&mut self, buf: &[u8]) {
        self.read_buffer.extend_from_slice(buf);
    }

    fn try_read<T: DeserializeOwned>(&mut self) -> ::Res<Option<T>> {
        if self.read_len == 0 && !self.try_read_header()? {
            return Ok(None);
        }
        if self.read_len > self.read_buffer.len() {
            return Ok(None);
        }

        let result = deserialise_from(&mut Cursor::new(&self.read_buffer))?;
        self.mark_read();
        Ok(Some(result))
    }

    fn try_read_header(&mut self) -> ::Res<bool> {
        let u32_size = mem::size_of::<u32>();
        if self.read_buffer.len() < u32_size {
            return Ok(false);
        }

        self.read_len = Cursor::new(&self.read_buffer).read_u32::<LittleEndian>()? as usize;

        if self.read_len > self.max_payload_size {
            return Err(SocketError::PayloadSizeProhibitive);
        }

        self.read_buffer = self.read_buffer[u32_size..].to_owned();
        Ok(true)
    }

    /// Shift read buffer and reset buffer to read length.
    fn mark_read(&mut self) {
        self.read_buffer = self.read_buffer[self.read_len..].to_owned();
        self.read_len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidsafe_utilities::serialisation::serialise;

    mod write_len_delimited {
        use super::*;

        #[test]
        fn it_writes_4_byte_data_length() {
            let buf = unwrap!(write_len_delimited(vec![1u8, 2, 3]));

            let exp_serialised = unwrap!(serialise(&vec![1u8, 2, 3]));

            assert_eq!(usize::from(buf[0]), exp_serialised.len());
        }
    }
}
