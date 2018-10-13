extern crate maidsafe_utilities;
extern crate mio;
extern crate socket_collection;
#[macro_use]
extern crate unwrap;

// use maidsafe_utilities::log;
use maidsafe_utilities::thread::{self, Joiner};
use mio::*;
use socket_collection::UdpSock;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

#[test]
fn udp_peers_connected_huge_data_exchange() {
    udp_peers_huge_data_exchange_impl(true);
}

#[test]
fn udp_peers_not_connected_huge_data_exchange() {
    udp_peers_huge_data_exchange_impl(false);
}

fn udp_peers_huge_data_exchange_impl(should_connect: bool) {
    const ITERATIONS: usize = 200;
    const DATA_SIZE: usize = 10 * 1024;
    const UDP0: Token = Token(0);
    const UDP1: Token = Token(1);

    let addr0 = unwrap!("127.0.0.1:0".parse());
    let addr1 = unwrap!("127.0.0.1:0".parse());

    let mut udp0 = unwrap!(UdpSock::bind(&addr0));
    let mut udp1 = unwrap!(UdpSock::bind(&addr1));

    // Actual addresses with valid ports by OS
    let addr0 = unwrap!(udp0.local_addr());
    let addr1 = unwrap!(udp1.local_addr());

    // Should be done after binding both otherwise over localhost linux can detect there's no UDP
    // socket bound to represent the peer being connected to and results in connect-error
    if should_connect {
        unwrap!(udp0.connect(&addr1));
        unwrap!(udp1.connect(&addr0));
    }

    let poll = unwrap!(Poll::new());

    unwrap!(poll.register(
        &udp0,
        UDP0,
        Ready::readable() | Ready::writable(),
        PollOpt::edge(),
    ));

    unwrap!(poll.register(
        &udp1,
        UDP1,
        Ready::readable() | Ready::writable(),
        PollOpt::edge(),
    ));

    let (tx, rx) = mpsc::channel();
    let wouldblocked = Arc::new(AtomicBool::new(false));
    let wouldblocked_cloned = wouldblocked.clone();

    let _j = thread::named("UDP0 sender", move || {
        let data = vec![255u8; DATA_SIZE];
        for _ in 0..ITERATIONS {
            match if should_connect {
                udp0.write(Some((data.clone(), 0)))
            } else {
                udp0.write_to(Some((data.clone(), addr1, 0)))
            } {
                Ok(true) => (),
                Ok(false) => {
                    wouldblocked.store(true, Ordering::SeqCst);
                    let _ = rx.recv_timeout(Duration::from_millis(50));
                    wouldblocked.store(false, Ordering::SeqCst);
                }
                Err(e) => panic!("UDP0 Error in send: {:?}", e),
            }
        }
    });

    let mut events = Events::with_capacity(1024);
    let expected_data = vec![255; DATA_SIZE];
    let mut iterations = 0;
    'event_loop: loop {
        let total_events = unwrap!(poll.poll(&mut events, Some(Duration::from_secs(3))));
        if total_events == 0 {
            assert!(events.is_empty());
            // Since UDP is lossy, there is no guarantee that all sent data will received (i.e.,
            // not lost). Assert that we have at-least got the majority of it though. Since it's
            // highly unlikely we would not receive a single event by the timeout, it probably
            // means there's none left now and we should break the event loop.
            assert!(iterations > ITERATIONS / 2);
            break;
        }

        for event in events.iter() {
            match event.token() {
                UDP0 => if event.kind().is_writable() && wouldblocked_cloned.load(Ordering::SeqCst)
                {
                    unwrap!(tx.send(()));
                },
                UDP1 => {
                    if !event.kind().is_readable() {
                        // Spurious wake
                        continue;
                    }
                    loop {
                        match if should_connect {
                            udp1.read::<Vec<u8>>()
                        } else {
                            udp1.read_frm::<Vec<u8>>().map(|opt| {
                                opt.map(|(d, peer)| {
                                    assert_eq!(peer, addr0);
                                    d
                                })
                            })
                        } {
                            Ok(Some(d)) => {
                                if d.len() < DATA_SIZE {
                                    panic!(
                                        "UDP1 Rxd {}B ;; expected {}B ;; Partial datagram rxd !",
                                        d.len(),
                                        DATA_SIZE
                                    )
                                } else if d.len() > DATA_SIZE {
                                    panic!(
                                        "UDP1 Rxd {}B ;; expected {}B ;; Bloated datagram rxd !",
                                        d.len(),
                                        DATA_SIZE
                                    )
                                }
                                // assert_eq!() will produce a huge log on failure, so using
                                // assert!() instead
                                assert!(d == expected_data);
                                iterations += 1;
                                if iterations == ITERATIONS {
                                    break 'event_loop;
                                }
                            }
                            Ok(None) => {
                                break;
                            }
                            Err(e) => panic!("UDP1 errored in Read: {:?}", e),
                        }
                    }
                }
                x => unreachable!("{:?}", x),
            }
        }
    }
}
