use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use libudt4_sys::{EASYNCRCV, EASYNCSND};
use maidsafe_utilities::serialisation::{deserialise_from, serialise_into};
use mio::{self, Evented, Poll, PollOpt, Ready, Token};
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::io::{self, Cursor, ErrorKind};
use std::net::SocketAddr;
use std::time::Instant;
use std::{self, mem};
use udt_extern::{SocketFamily, SocketType, UdtOpts, UdtSocket};
use {
    Handle, Priority, SocketError, UdpSock, MAX_MSG_AGE_SECS, MAX_PAYLOAD_SIZE, MSG_DROP_PRIORITY,
};

const UDP_SNDBUF_SIZE: i32 = 512 * 1024;
const UDP_RCVBUF_SIZE: i32 = 512 * 1024;

pub struct UdtSock {
    inner: Option<Inner>,
}

impl UdtSock {
    pub fn wrap_std_sock(udp_sock: std::net::UdpSocket, handle: Handle) -> ::Res<Self> {
        let stream = UdtSocket::new(SocketFamily::AFInet, SocketType::Stream)?;

        // Since we are supplying the UDP socket we don't want the UDT to make assumptions about it
        // as the default at the time of this comment was 1MiB
        stream.setsockopt(UdtOpts::UDP_SNDBUF, UDP_SNDBUF_SIZE)?;
        stream.setsockopt(UdtOpts::UDP_RCVBUF, UDP_RCVBUF_SIZE)?;
        // Make connect and reads non-blocking
        stream.setsockopt(UdtOpts::UDT_RCVSYN, false)?;
        // Make writes non-blocking
        stream.setsockopt(UdtOpts::UDT_SNDSYN, false)?;
        // Enable rendezvous mode for simultaneous connect. This is necessary to bind an existing
        // UDP socket
        stream.setsockopt(UdtOpts::UDT_RENDEZVOUS, true)?;

        stream.bind_from(udp_sock)?;

        Ok(UdtSock {
            inner: Some(Inner {
                stream,
                handle,
                read_buffer: Vec::new(),
                read_len: 0,
                write_queue: BTreeMap::new(),
                current_write: None,
            }),
        })
    }

    pub fn wrap_mio_sock(udp_sock: mio::net::UdpSocket, handle: Handle) -> ::Res<Self> {
        UdtSock::wrap_std_sock(mio_to_std_udp_sock(udp_sock), handle)
    }

    pub fn wrap_udp_sock(udp_sock: UdpSock, handle: Handle) -> ::Res<Self> {
        UdtSock::wrap_mio_sock(udp_sock.into_underlying_sock()?, handle)
    }

    pub fn connect(&self, addr: &SocketAddr) -> ::Res<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.stream.connect(*addr)?)
    }

    pub fn peer_addr(&self) -> ::Res<SocketAddr> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.stream.getpeername()?)
    }

    pub fn take_error(&self) -> ::Res<Option<io::Error>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        // getlasterror() //FIXME: needs mapped
        // Ok(inner.stream.take_error()?)
        unimplemented!()
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

impl Default for UdtSock {
    fn default() -> Self {
        UdtSock { inner: None }
    }
}

impl Evented for UdtSock {
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
    stream: UdtSocket,
    handle: Handle,
    read_buffer: Vec<u8>,
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
    fn read<T: DeserializeOwned>(&mut self) -> ::Res<Option<T>> {
        if let Some(message) = self.read_from_buffer()? {
            return Ok(Some(message));
        }

        // the mio reading window is max at 64k (64 * 1024)
        const BUF_LEN: usize = 64 * 1024;
        let mut buffer = [0; BUF_LEN];
        let mut is_something_read = false;

        loop {
            match self.stream.recv(&mut buffer, BUF_LEN) {
                Ok(bytes_rxd) => {
                    if bytes_rxd < 0 {
                        return Err(SocketError::UdtNegativeBytesRead(bytes_rxd));
                    }
                    let bytes_rxd = bytes_rxd as usize;
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
                    self.read_buffer.extend_from_slice(&buffer[0..bytes_rxd]);
                    is_something_read = true;
                }
                Err(error) => {
                    return if error.err_code == EASYNCRCV {
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

    fn read_from_buffer<T: DeserializeOwned>(&mut self) -> ::Res<Option<T>> {
        let u32_size = mem::size_of::<u32>();

        if self.read_len == 0 {
            if self.read_buffer.len() < u32_size {
                return Ok(None);
            }

            self.read_len = Cursor::new(&self.read_buffer).read_u32::<LittleEndian>()? as usize;

            if self.read_len > MAX_PAYLOAD_SIZE {
                return Err(SocketError::PayloadSizeProhibitive);
            }

            self.read_buffer = self.read_buffer[u32_size..].to_owned();
        }

        if self.read_len > self.read_buffer.len() {
            return Ok(None);
        }

        let result = deserialise_from(&mut Cursor::new(&self.read_buffer))?;

        self.read_buffer = self.read_buffer[self.read_len..].to_owned();
        self.read_len = 0;

        Ok(Some(result))
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
            let mut data = Cursor::new(Vec::with_capacity(mem::size_of::<u32>()));

            let _ = data.write_u32::<LittleEndian>(0);

            serialise_into(&msg, &mut data)?;

            let len = data.position() - mem::size_of::<u32>() as u64;
            data.set_position(0);
            data.write_u32::<LittleEndian>(len as u32)?;

            let entry = self
                .write_queue
                .entry(priority)
                .or_insert_with(|| VecDeque::with_capacity(10));
            entry.push_back((Instant::now(), data.into_inner()));
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

            match self.stream.send(&data) {
                Ok(bytes_txd) => {
                    if bytes_txd < 0 {
                        return Err(SocketError::UdtNegativeBytesWrite(bytes_txd));
                    }
                    let bytes_txd = bytes_txd as usize;
                    if bytes_txd < data.len() {
                        self.current_write = Some(data[bytes_txd..].to_owned());
                    }
                }
                Err(error) => {
                    if error.err_code == EASYNCSND {
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
        _poll: &Poll,
        token: Token,
        interest: Ready,
        _opts: PollOpt,
    ) -> io::Result<()> {
        Ok(self
            .handle
            .register(self.stream, token, interest)
            .map_err(|e| into_io_error(Some(e), ""))?)
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        self.register(poll, token, interest, opts)
    }

    fn deregister(&self, _poll: &Poll) -> io::Result<()> {
        Ok(self
            .handle
            .deregister(self.stream)
            .map_err(|e| into_io_error(Some(e), ""))?)
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Err(e) = self.stream.close() {
            debug!("Error closing UDT socket: {:?}", e);
        }
    }
}

fn into_io_error<E: Debug>(e: Option<E>, m: &str) -> io::Error {
    let mut err_msg = if let Some(e) = e {
        format!("Error: {:?}", e)
    } else {
        "".to_string()
    };
    // FIXME: do better
    let opt_details = format!(";; [Optional details: {}]", m);
    err_msg.push_str(if m.is_empty() { "" } else { &opt_details });

    io::Error::new(ErrorKind::Other, err_msg)
}

#[allow(unsafe_code)]
#[cfg(target_family = "unix")]
fn mio_to_std_udp_sock(socket: mio::net::UdpSocket) -> std::net::UdpSocket {
    use std::os::unix::io::{FromRawFd, IntoRawFd};
    unsafe { FromRawFd::from_raw_fd(socket.into_raw_fd()) }
}

#[allow(unsafe_code)]
#[cfg(target_family = "windows")]
fn mio_to_std_udp_sock(_socket: mio::net::UdpSocket) -> std::net::UdpSocket {
    // FIXME: Currently mio does not have this facility: https://github.com/carllerche/mio/pull/859
    //
    // use std::os::windows::io::{FromRawSocket, IntoRawSocket};
    // unsafe { FromRawFd::from_raw_socket(socket.into_raw_socket()) }
    unimplemented!(
        "Currently mio does not have this facility: https://github.com/carllerche/mio/pull/859"
    );
}
