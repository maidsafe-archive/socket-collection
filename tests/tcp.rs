#[macro_use]
extern crate unwrap;
extern crate socket_collection;
#[macro_use]
extern crate net_literals;
extern crate mio;
extern crate rand;

use mio::net::TcpListener;
use mio::{Events, Poll, PollOpt, Ready, Token};
use rand::RngCore;
use socket_collection::{SocketError, TcpSock};

#[test]
fn data_transfer_up_to_2_mb() {
    const SOCKET1_TOKEN: Token = Token(0);
    const SOCKET2_TOKEN: Token = Token(1);
    const LISTENER_TOKEN: Token = Token(2);
    let evloop = unwrap!(Poll::new());

    let listener = unwrap!(TcpListener::bind(&addr!("127.0.0.1:0")));
    let listener_addr = unwrap!(listener.local_addr());

    let mut sock1 = unwrap!(TcpSock::connect(&listener_addr));
    let mut sock2 = None;

    unwrap!(evloop.register(&sock1, SOCKET1_TOKEN, Ready::writable(), PollOpt::edge(),));
    unwrap!(evloop.register(
        &listener,
        LISTENER_TOKEN,
        Ready::readable(),
        PollOpt::edge(),
    ));

    let data_len = 1024 * 1024 * 2 - 40;
    let out_data = random_vec(data_len);
    let mut data_sent = false;
    let mut in_data = Vec::with_capacity(data_len);

    let mut events = Events::with_capacity(16);
    'event_loop: loop {
        unwrap!(evloop.poll(&mut events, None));
        for ev in events.iter() {
            match ev.token() {
                LISTENER_TOKEN => {
                    let (client_sock, _client_addr) = unwrap!(listener.accept());
                    unwrap!(evloop.register(
                        &client_sock,
                        SOCKET2_TOKEN,
                        Ready::readable(),
                        PollOpt::edge(),
                    ));
                    sock2 = Some(TcpSock::wrap(client_sock));
                }
                SOCKET1_TOKEN => {
                    if !data_sent {
                        let _ = unwrap!(sock1.write(Some((out_data.clone(), 1))));
                        data_sent = true;
                    } else {
                        let _ = unwrap!(sock1.write::<Vec<u8>>(None));
                    }
                }
                SOCKET2_TOKEN => loop {
                    let res: Result<Option<Vec<u8>>, SocketError> = unwrap!(sock2.as_mut()).read();
                    match res {
                        Ok(Some(data)) => in_data.extend_from_slice(&data),
                        Ok(None) => break,
                        Err(e) => panic!("Data read failed: {}", e),
                    }
                    if in_data.len() == data_len {
                        assert_eq!(in_data, out_data);
                        break 'event_loop;
                    }
                },
                _ => panic!("Unexpected event"),
            }
        }
    }
}

#[allow(unsafe_code)]
pub fn random_vec(size: usize) -> Vec<u8> {
    let mut ret = Vec::with_capacity(size);
    unsafe { ret.set_len(size) };
    rand::thread_rng().fill_bytes(&mut ret[..]);
    ret
}
