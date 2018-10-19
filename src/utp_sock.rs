use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use maidsafe_utilities::serialisation::{deserialise_from, serialise_into};
use mio::net::UdpSocket;
use mio::{Evented, Poll, PollOpt, Ready, Registration, SetReadiness, Token};
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Cursor, ErrorKind};
use std::mem;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;
use utp::{UtpCallbackType, UtpContext, UtpError, UtpSocket, UtpState};
use {Priority, SocketError, MAX_MSG_AGE_SECS, MAX_PAYLOAD_SIZE, MSG_DROP_PRIORITY};

// TODO(povilas): there's a bunch of duplicate code from udp.rs - reuse it.

/// Asynchronous uTP socket backed by mio UdpSocket.
pub struct UtpSock {
    inner: Option<Inner>,
}

impl UtpSock {
    /// Create new uTP socket bound to given local address.
    ///
    /// It also registers the underlying UDP socket on mio event loop with given token.
    ///
    /// # Args
    ///
    /// `udp_sock_token` - under the hood `UtpSock` drives mio UDP socket: it receives UDP packets
    ///                    and tries to parse and interpret them as uTP packets.
    ///                    This argument is mio token used to notify when UDP socket becomes
    ///                    readable. `UtpSocket` does not know when that happens, so you should
    ///                    notify it via `UtpSock::udp_readable()`.
    /// `timer_token` - every 500 ms uTP checks for lost packets, timedout connections, etc.
    ///                 `UtpSock::bind()` will setup and register the timer, but it relies on mio
    ///                 to run it. Also, when the timer becomes available, call
    ///                 `UtpSock::timer_tick()`.
    pub fn bind(addr: &SocketAddr, udp_sock_token: Token, timer_token: Token) -> ::Res<Self> {
        Ok(Self {
            inner: Some(Inner::bind(addr, udp_sock_token)?),
        })
    }

    /// Initiates connection to given address. Returns immediately.
    /// You should check if connection was successful when `UtpSock` becomes writable.
    pub fn connect(&mut self, addr: &SocketAddr) -> ::Res<()> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        let utp_sock = inner.utp_ctx.connect(*addr)?;
        inner.utp_sock = Some(utp_sock);
        Ok(())
    }

    /// Check if socket is already connected.
    pub fn connected(&self) -> bool {
        self.inner.as_ref().map_or(false, |inner| inner.connected())
    }

    /// Returns the local address this uTP socket is bound to.
    pub fn local_addr(&self) -> ::Res<SocketAddr> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.udp_sock.local_addr()?)
    }

    /// Sets time-to-live value for the underlying UDP socket.
    pub fn set_ttl(&self, ttl: u32) -> ::Res<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.udp_sock.set_ttl(ttl)?)
    }

    /// Returns UDP socket TTL value.
    pub fn ttl(&self) -> ::Res<u32> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.udp_sock.ttl()?)
    }

    /// Get the value of the `SO_ERROR` option on underlying UDP socket.
    ///
    /// This will retrieve the stored error in the underlying socket, clearing the field in the
    /// process. This can be useful for checking errors between calls.
    pub fn take_error(&self) -> ::Res<Option<io::Error>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(SocketError::UninitialisedSocket)?;
        Ok(inner.udp_sock.take_error()?)
    }

    /// Should be called when the underlying UDP socket becomes readable.
    /// This method will drain the UDP socket and feed the packets for uTP processing.
    pub fn udp_readable(&mut self) -> ::Res<()> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.udp_readable()
    }

    /// Ticks the internal uTP timer. Should be called every 500ms. Checks for connection timeouts
    /// and unacked packets.
    pub fn timer_tick(&mut self) -> ::Res<()> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.timer_tick();
        Ok(())
    }

    /// Read message from the socket. Call this from inside the `ready` handler.
    ///
    /// # Returns
    ///
    /// - Ok(Some(data)): data has been successfully read from the socket
    /// - Ok(None):       there is not enough data in the socket. Call `read` again in the next
    ///                   invocation of the `ready` handler.
    /// - Err(error):     there was an error reading from the socket.
    pub fn read<T: DeserializeOwned + Serialize>(&mut self) -> ::Res<Option<T>> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.read()
    }

    /// Write a message to the socket.
    ///
    /// # Returns
    ///   - Ok(true):   the message has been successfully written.
    ///   - Ok(false):  the message has been queued, but not yet fully written.
    ///                 will be attempted in the next write schedule.
    ///   - Err(error): there was an error while writing to the socket.
    pub fn write<T: Serialize>(&mut self, msg: Option<(T, Priority)>) -> ::Res<bool> {
        let inner = self
            .inner
            .as_mut()
            .ok_or(SocketError::UninitialisedSocket)?;
        inner.write(msg)
    }
}

impl Default for UtpSock {
    fn default() -> Self {
        Self { inner: None }
    }
}

impl Evented for UtpSock {
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

/// Data shared with `UtpSocket` callbacks. Used as a bridge between `utp::UtpSocket` and our
/// `UtpSock`.
struct SharedWithUtp {
    /// Buffered payload of uTP data packets.
    read_buffer: Vec<u8>,
    errors: Vec<SocketError>,
    connected: bool,
}

impl SharedWithUtp {
    fn new() -> Self {
        Self {
            read_buffer: Vec::new(),
            errors: Vec::new(),
            connected: false,
        }
    }
}

struct UserData {
    udp_sock: Arc<UdpSocket>,
    set_readiness: SetReadiness,
    shared: Rc<RefCell<SharedWithUtp>>,
}

/// `UtpSock` inner struct that is always set compared to `UtpSock` whose default fields are `None`.
struct Inner {
    /// Underlying system socket.
    udp_sock: Arc<UdpSocket>,
    /// Unconnected sockets don't have `UtpSocket` associated.
    utp_sock: Option<UtpSocket>,
    utp_ctx: UtpContext<UserData>,
    registration: Registration,
    udp_sock_token: Token,

    /// Buffered outgoing data that will be serialized into uTP packets and sent over UDP.
    write_queue: BTreeMap<Priority, VecDeque<(Instant, Vec<u8>)>>,
    /// In case of partial send, we hold a buffer to send.
    current_write: Option<Vec<u8>>,
    /// Length delimited received data size in bytes.
    read_len: usize,

    /// Data shared between uTP callbacks and `Inner`.
    shared: Rc<RefCell<SharedWithUtp>>,
}

impl Inner {
    fn bind(addr: &SocketAddr, udp_sock_token: Token) -> ::Res<Self> {
        let udp_sock = Arc::new(UdpSocket::bind(addr)?);
        let (registration, set_readiness) = Registration::new2();

        let shared = Rc::new(RefCell::new(SharedWithUtp::new()));

        let user_data = UserData {
            udp_sock: Arc::clone(&udp_sock),
            set_readiness,
            shared: Rc::clone(&shared),
        };
        let utp_ctx = make_utp_ctx(user_data);

        Ok(Self {
            udp_sock,
            utp_sock: None,
            utp_ctx,
            registration,
            udp_sock_token,
            write_queue: BTreeMap::new(),
            current_write: None,
            read_len: 0,
            shared,
        })
    }
}

impl Inner {
    /// Check if uTP socket got connected.
    pub fn connected(&self) -> bool {
        self.shared.borrow().connected
    }

    /// Read message from the socket. Call this from inside the `ready` handler.
    ///
    /// # Returns:
    ///
    /// - Ok(Some(data)): data has been successfully read from the socket.
    /// - Ok(None):       there is not enough data in the socket. Call `read`
    ///                   again in the next invocation of the `ready` handler.
    /// - Err(error):     there was an error reading from the socket.
    fn read<T: DeserializeOwned + Serialize>(&mut self) -> ::Res<Option<T>> {
        // TODO(povilas): isn't u32 supposed to always be 4 bytes long?
        let u32_size = mem::size_of::<u32>();

        let mut shared = self.shared.borrow_mut();
        if self.read_len == 0 {
            if shared.read_buffer.len() < u32_size {
                return Ok(None);
            }

            self.read_len = Cursor::new(&shared.read_buffer).read_u32::<LittleEndian>()? as usize;
            if self.read_len > MAX_PAYLOAD_SIZE {
                return Err(SocketError::PayloadSizeProhibitive);
            }

            // TODO(povilas): doesn't this involve data relocation, hence copying? V
            shared.read_buffer = shared.read_buffer[u32_size..].to_owned();
        }

        if self.read_len > shared.read_buffer.len() {
            return Ok(None);
        }

        let result = deserialise_from(&mut Cursor::new(&shared.read_buffer))?;

        // shift read buffer right
        // TODO(povilas): doesn't this involve data relocation, hence copying? V
        shared.read_buffer = shared.read_buffer[self.read_len..].to_owned();
        self.read_len = 0;

        Ok(Some(result))
    }

    /// Write a message to the socket.
    ///
    /// # Args
    ///
    /// `msg`: value to be sent over uTP connection. If `None`, retries sending previously buffered
    ///     data.
    ///
    /// # Returns
    ///   - Ok(true):   the message has been successfully written.
    ///   - Ok(false):  the message has been queued, but not yet fully written.
    ///                 will be attempted in the next write schedule.
    ///   - Err(error): there was an error while writing to the socket.
    fn write<T: Serialize>(&mut self, msg: Option<(T, Priority)>) -> ::Res<bool> {
        self.drop_expired();
        if let Some((msg, priority)) = msg {
            self.enqueue_data(msg, priority)?;
        }
        self.flush_write_until_would_block()
    }

    fn timer_tick(&mut self) {
        self.utp_ctx.check_timeouts();
    }

    fn udp_readable(&mut self) -> ::Res<()> {
        // the mio reading window is max at 64k (64 * 1024)
        const BUF_LEN: usize = 64 * 1024;
        let mut buf = [0; BUF_LEN];

        loop {
            match self.udp_sock.recv_from(&mut buf) {
                Ok((bytes_read, sender_addr)) => {
                    if self
                        .utp_ctx
                        .process_udp(&buf[..bytes_read], sender_addr)
                        .is_err()
                    {
                        debug!("UDP datagram is illegal uTP packet");
                    }
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock || e.kind() == ErrorKind::Interrupted {
                        self.utp_ctx.ack_packets();
                        break;
                    } else {
                        return Err(e.into());
                    }
                }
            }
        }
        Ok(())
    }

    fn flush_write_until_would_block(&mut self) -> ::Res<bool> {
        // Make sure we are connected before we can send the data.
        let utp_sock: &UtpSocket = self
            .utp_sock
            .as_ref()
            .ok_or(SocketError::Io(io::ErrorKind::BrokenPipe.into()))?;

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

            match utp_sock.send(&data) {
                Ok(bytes_txd) => {
                    if bytes_txd < data.len() {
                        self.current_write = Some(data[bytes_txd..].to_owned());
                    }
                }
                Err(error) => {
                    if error == UtpError::WouldBlock {
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
        let entry = self
            .write_queue
            .entry(priority)
            .or_insert_with(|| VecDeque::with_capacity(10));
        entry.push_back((Instant::now(), buf));
        Ok(())
    }

    fn drop_expired(&mut self) {
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
        self.udp_sock.register(
            poll,
            self.udp_sock_token,
            Ready::readable(),
            PollOpt::edge(),
        )?;
        self.registration.register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        self.udp_sock.reregister(
            poll,
            self.udp_sock_token,
            Ready::readable(),
            PollOpt::edge(),
        )?;
        self.registration.reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        self.udp_sock.deregister(poll)?;
        self.registration.deregister(poll)
    }
}

fn make_utp_ctx(data: UserData) -> UtpContext<UserData> {
    let mut utp = UtpContext::new(data);
    utp.set_callback(
        UtpCallbackType::OnError,
        Box::new(|args| {
            args.user_data()
                .shared
                .borrow_mut()
                .errors
                .push(SocketError::Io(args.error()));
            0
        }),
    );
    utp.set_callback(
        UtpCallbackType::Sendto,
        Box::new(|args| {
            if let Some(addr) = args.address() {
                let sock = &args.user_data().udp_sock;
                match sock.send_to(args.buf(), &addr) {
                    Ok(bytes_sent) => debug_assert_eq!(args.buf().len(), bytes_sent),
                    Err(e) => if e.kind() != io::ErrorKind::WouldBlock {
                        args.user_data()
                            .shared
                            .borrow_mut()
                            .errors
                            .push(SocketError::Io(e));
                    },
                }
            }
            0
        }),
    );
    utp.set_callback(
        UtpCallbackType::OnRead,
        Box::new(|mut args| {
            // Note, data copying occurs here.
            args.user_data()
                .shared
                .borrow_mut()
                .read_buffer
                .extend_from_slice(args.buf());
            if let Err(e) = args
                .user_data()
                .set_readiness
                .set_readiness(Ready::readable())
            {
                error!("Failed to awake UtpSock with writable state: {}", e);
            }
            args.ack_data();
            0
        }),
    );
    utp.set_callback(
        UtpCallbackType::OnStateChange,
        Box::new(move |args| {
            if args.state() == UtpState::Connected {
                args.user_data().shared.borrow_mut().connected = true;
            }
            if args.state() == UtpState::Connected || args.state() == UtpState::Writable {
                if let Err(e) = args
                    .user_data()
                    .set_readiness
                    .set_readiness(Ready::writable())
                {
                    error!("Failed to awake UtpSock with readable state: {}", e);
                }
            }
            0
        }),
    );
    utp
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
