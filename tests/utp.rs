#[macro_use]
extern crate unwrap;
extern crate socket_collection;
#[macro_use]
extern crate net_literals;
extern crate mio;
extern crate rand;

use mio::{Events, Poll, PollOpt, Ready, Token};
use rand::RngCore;
use socket_collection::UtpSock;

#[test]
fn connection() {
    const SOCKET1_TOKEN: Token = Token(0);
    const SOCKET2_TOKEN: Token = Token(1);
    const UDP1_TOKEN: Token = Token(2);
    const UDP2_TOKEN: Token = Token(3);
    let evloop = unwrap!(Poll::new());

    let mut sock1 = unwrap!(UtpSock::bind(&addr!("127.0.0.1:0"), UDP1_TOKEN, Token(666)));
    let addr1 = unwrap!(sock1.local_addr());

    let mut sock2 = unwrap!(UtpSock::bind(&addr!("127.0.0.1:0"), UDP2_TOKEN, Token(666)));
    let addr2 = unwrap!(sock2.local_addr());

    unwrap!(sock1.connect(&addr2));
    unwrap!(sock2.connect(&addr1));

    unwrap!(evloop.register(&sock1, SOCKET1_TOKEN, Ready::writable(), PollOpt::level(),));
    unwrap!(evloop.register(&sock2, SOCKET2_TOKEN, Ready::writable(), PollOpt::level(),));

    let mut sock1_connected = false;
    let mut sock2_connected = false;

    let mut events = Events::with_capacity(16);
    while !(sock1_connected & sock2_connected) {
        unwrap!(evloop.poll(&mut events, None));
        for ev in events.iter() {
            match ev.token() {
                SOCKET1_TOKEN => sock1_connected = true,
                SOCKET2_TOKEN => sock2_connected = true,
                UDP1_TOKEN => unwrap!(sock1.udp_readable()),
                UDP2_TOKEN => unwrap!(sock2.udp_readable()),
                _ => panic!("Unexpected event"),
            }
        }
    }
}

#[test]
fn data_transfer_up_to_2_mb() {
    const SOCKET1_TOKEN: Token = Token(0);
    const SOCKET2_TOKEN: Token = Token(1);
    const UDP1_TOKEN: Token = Token(2);
    const UDP2_TOKEN: Token = Token(3);
    let evloop = unwrap!(Poll::new());

    let mut sock1 = unwrap!(UtpSock::bind(&addr!("127.0.0.1:0"), UDP1_TOKEN, Token(666)));
    let addr1 = unwrap!(sock1.local_addr());

    let mut sock2 = unwrap!(UtpSock::bind(&addr!("127.0.0.1:0"), UDP2_TOKEN, Token(666)));
    let addr2 = unwrap!(sock2.local_addr());

    unwrap!(sock1.connect(&addr2));
    unwrap!(sock2.connect(&addr1));

    unwrap!(evloop.register(&sock1, SOCKET1_TOKEN, Ready::writable(), PollOpt::edge(),));
    unwrap!(evloop.register(&sock2, SOCKET2_TOKEN, Ready::readable(), PollOpt::level(),));

    let out_data = random_vec(1024 * 1024 * 2 - 40);
    let mut data_sent = false;

    let mut events = Events::with_capacity(16);
    'event_loop: loop {
        unwrap!(evloop.poll(&mut events, None));
        for ev in events.iter() {
            match ev.token() {
                SOCKET1_TOKEN => {
                    if !data_sent {
                        let _ = unwrap!(sock1.write(Some((out_data.clone(), 1))));
                        data_sent = true;
                    } else {
                        let _ = unwrap!(sock1.write::<Vec<u8>>(None));
                    }
                }
                SOCKET2_TOKEN => {
                    let in_data: Option<Vec<u8>> = unwrap!(sock2.read());
                    if let Some(in_data) = in_data {
                        assert_eq!(in_data.len(), out_data.len());
                        assert_eq!(in_data, out_data);
                        break 'event_loop;
                    }
                }
                UDP1_TOKEN => unwrap!(sock1.udp_readable()),
                UDP2_TOKEN => unwrap!(sock2.udp_readable()),
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
