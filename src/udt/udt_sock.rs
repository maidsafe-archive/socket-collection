use {SocketError, Priority, MAX_MSG_AGE_SECS, MAX_PAYLOAD_SIZE, MSG_DROP_PRIORITY};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use libudt4_sys::{EASYNCRCV, EASYNCSND};
use maidsafe_utilities::serialisation::{deserialise_from, serialise_into};
use mio::udp::UdpSocket;
use mio::{Evented, Poll, PollOpt, Ready, Token};
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::io::{self, Cursor, ErrorKind, Read, Write};
use std::mem;
use std::net::SocketAddr;
use std::sync::{Mutex, Weak};
use std::time::Instant;
use udt_extern::{Epoll, EpollEvents, SocketFamily, SocketType, UDT_EPOLL_ERR, UDT_EPOLL_IN, UDT_EPOLL_OUT, UdtOpts, UdtSocket};

pub struct UdtSock {
    inner: Option<Inner>,
}

impl UdtSock {
    pub fn wrap(udp_sock: UdpSocket, epoll: Weak<Mutex<Epoll>>) -> ::Res<Self> {
        let stream = UdtSocket::new(SocketFamily::AFInet, SocketType::Stream)?;
        // FIXME: Check code in udt-rs to see if `bind_from` is really consuming the socket
        // else it'll be a problem
        stream.bind_from(mio_to_std_udp_sock(udp_sock))?;

        // Make connect and reads non-blocking
        stream.setsockopt(UdtOpts::UDT_RCVSYN, false)?;
        // Make writes non-blocking
        stream.setsockopt(UdtOpts::UDT_SNDSYN, false)?;
        // Enable rendezvous mode for simultaneous connect
        stream.setsockopt(UdtOpts::UDT_RENDEZVOUS, true)?;

        Ok(UdtSock {
            inner: Some(Inner {
                stream,
                read_buffer: Vec::new(),
                read_len: 0,
                write_queue: BTreeMap::new(),
                current_write: None,
                epoll,
            }),
        })
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
    //                 Write event is already scheduled for next time.
    //   - Err(error): there was an error while writing to the socket.
    pub fn write<T: Serialize>(
        &mut self,
        poll: &Poll,
        token: Token,
        msg: Option<(T, Priority)>,
    ) -> ::Res<bool> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.write(poll, token, msg)
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
    read_buffer: Vec<u8>,
    read_len: usize,
    write_queue: BTreeMap<Priority, VecDeque<(Instant, Vec<u8>)>>,
    current_write: Option<Vec<u8>>,
    epoll: Weak<Mutex<Epoll>>,
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
                Ok(bytes_read) => {
                    let bytes_read = bytes_read as usize;
                    if bytes_read == 0 {
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
                    self.read_buffer.extend_from_slice(&buffer[0..bytes_read]);
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
    //                 Write event is already scheduled for next time.
    //   - Err(error): there was an error while writing to the socket.
    fn write<T: Serialize>(
        &mut self,
        poll: &Poll,
        token: Token,
        msg: Option<(T, Priority)>,
    ) -> ::Res<bool> {
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

        if self.current_write.is_none() {
            let (key, (_time_stamp, data), empty) = match self.write_queue.iter_mut().next() {
                Some((key, queue)) => (*key, unwrap!(queue.pop_front()), queue.is_empty()),
                None => return Ok(true),
            };
            if empty {
                let _ = self.write_queue.remove(&key);
            }
            self.current_write = Some(data);
        }

        if let Some(data) = self.current_write.take() {
            match self.stream.send(&data) {
                Ok(bytes_txd) => {
                    let bytes_txd = bytes_txd as usize;
                    if bytes_txd < data.len() {
                        self.current_write = Some(data[bytes_txd..].to_owned());
                    }
                }
                Err(error) => {
                    if error.err_code == EASYNCSND {
                        self.current_write = Some(data);
                    } else {
                        return Err(From::from(error));
                    }
                }
            }
        }

        let done = self.current_write.is_none() && self.write_queue.is_empty();

        let event_set = if done {
            Ready::error() | Ready::hup() | Ready::readable()
        } else {
            Ready::error() | Ready::hup() | Ready::readable() | Ready::writable()
        };

        poll.reregister(self, token, event_set, PollOpt::edge())?;

        Ok(done)
    }
}

impl Evented for Inner {
    fn register(
        &self,
        _poll: &Poll,
        _token: Token,
        interest: Ready,
        _opts: PollOpt,
    ) -> io::Result<()> {
        let epoll = match self.epoll.upgrade() {
            Some(epoll) => epoll,
            None => return Err(into_io_error(Option::None::<io::Error>, "No UDT Epoll while registering !")),
        };

        let mut event_set = EpollEvents::empty();
        if interest.is_readable() {
            event_set.insert(UDT_EPOLL_IN)
        }
        if interest.is_writable() {
            event_set.insert(UDT_EPOLL_OUT)
        }
        event_set.insert(UDT_EPOLL_ERR);

        let mut locked_epoll = unwrap!(epoll.lock());
        locked_epoll.
            remove_usock(&self.stream).
            map_err(|e| into_io_error(Some(e), "While deregistering before registering"))?;
        Ok(locked_epoll.
            add_usock(&self.stream, Some(event_set)).
            map_err(|e| into_io_error(Some(e), "While registering after deregistering"))?)
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
        let epoll = match self.epoll.upgrade() {
            Some(epoll) => epoll,
            None => return Err(into_io_error(Option::None::<io::Error>, "No UDT Epoll while deregistering !")),
        };

        let locked_epoll = unwrap!(epoll.lock());
        Ok(locked_epoll.
            remove_usock(&self.stream).
            map_err(|e| into_io_error(Some(e), "While deregistering"))?)
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
    err_msg.push_str(if m.is_empty() {
        ""
    } else {
        &opt_details
    });

    io::Error::new(ErrorKind::Other, err_msg)
}

#[allow(unsafe_code)]
#[cfg(target_family = "unix")]
fn mio_to_std_udp_sock(socket: UdpSocket) -> ::std::net::UdpSocket {
    use std::os::unix::io::{FromRawFd, IntoRawFd};
    unsafe { FromRawFd::from_raw_fd(socket.into_raw_fd()) }
}

#[allow(unsafe_code)]
#[cfg(target_family = "windows")]
fn mio_to_std_udp_sock(_socket: UdpSocket) -> ::std::net::UdpSocket {
    use std::os::windows::io::{FromRawSocket, IntoRawSocket};
    unsafe { FromRawFd::from_raw_socket(socket.into_raw_socket()) }
}
