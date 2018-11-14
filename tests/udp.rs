extern crate maidsafe_utilities;
extern crate mio;
extern crate mio_extras;
extern crate socket_collection;
#[macro_use]
extern crate unwrap;
#[macro_use]
extern crate hamcrest2;

use hamcrest2::prelude::*;
use maidsafe_utilities::thread;
use mio::*;
use mio_extras::timer::Timer;
use socket_collection::UdpSock;
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
    const DATA_SIZE: usize = 9126; // max UDP datagram size on MacOS
    const UDP0: Token = Token(0);
    const UDP1: Token = Token(1);
    const TIMEOUT: Token = Token(2);

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

    let mut test_timeout = Timer::default();
    let _ = test_timeout.set_timeout(Duration::from_secs(5), ());
    unwrap!(poll.register(&test_timeout, TIMEOUT, Ready::readable(), PollOpt::edge(),));

    let (tx, rx) = mpsc::channel();
    let wouldblocked = Arc::new(AtomicBool::new(false));
    let wouldblocked_cloned = wouldblocked.clone();

    let _j = thread::named("UDP0 sender", move || {
        let data = vec![255u8; DATA_SIZE];
        for _i in 0..ITERATIONS {
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

            #[cfg(target_os = "macos")]
            {
                if _i % 50 == 0 {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    });

    let mut events = Events::with_capacity(1024);
    let expected_data = vec![255; DATA_SIZE];
    let mut iterations = 0;
    'event_loop: loop {
        let _ = unwrap!(poll.poll(&mut events, None));
        for event in events.iter() {
            match event.token() {
                UDP0 => if event.readiness().is_writable()
                    && wouldblocked_cloned.load(Ordering::SeqCst)
                {
                    unwrap!(tx.send(()));
                },
                UDP1 => {
                    if !event.readiness().is_readable() {
                        // Spurious wake
                        continue;
                    }
                    loop {
                        match if should_connect {
                            udp1.read::<Vec<u8>>()
                        } else {
                            udp1.read_frm::<Vec<u8>>().map(|opt| {
                                opt.map(|(d, peer)| {
                                    assert_that!(peer, eq(addr0));
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
                TIMEOUT => {
                    assert_that!(iterations, gt(ITERATIONS / 2));
                    break 'event_loop;
                }
                x => unreachable!("{:?}", x),
            }
        }
    }
}
