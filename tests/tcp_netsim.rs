#[macro_use]
extern crate unwrap;
extern crate futures;
extern crate mio;
extern crate netsim;
extern crate socket_collection;
extern crate tokio_core;

use futures::{Future, Stream};
use mio::{Events, Poll, PollOpt, Ready, Token};
use netsim::wire::{IntoIpPlug, IpPlug};
use netsim::{Ipv4AddrExt, Ipv4Range, Ipv4Route, Network};
use socket_collection::{SocketConfig, SocketError, TcpSock};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};
use std::sync::mpsc;
use tokio_core::reactor::Core;

#[cfg(target_os = "linux")]
#[test]
fn socket_timesout_when_remote_peer_does_not_respond_to_keep_alive_requests() {
    let mut evloop = Core::new().unwrap();
    let network = Network::new(&evloop.handle());
    let network_handle = network.handle();

    let (server_addr_tx, server_addr_rx) = mpsc::channel();
    let (client_done_tx, client_done_rx) = mpsc::channel();
    let (plug_client, plug_server) = IpPlug::new_pair();

    let mut syn_received = false;
    let mut ack_received = false;
    let plug_server = plug_server.filter(move |packet| {
        // Drop all incoming packets to server after TCP connection was established. So keepalive
        // packets comming from client will be dropped and client won't receive further ACKs
        let allow_packet = !(syn_received && ack_received);

        syn_received |= packet.to_ipv4_packet().and_then(|p| p.to_tcp_packet())
            .and_then(|p| Some(p.is_syn())).unwrap_or(false);
        ack_received |= packet.to_ipv4_packet().and_then(|p| p.to_tcp_packet())
            .and_then(|p| Some(p.is_ack())).unwrap_or(false);

        allow_packet
    }).into_ip_plug(&network_handle);

    let server_ip = Ipv4Addr::random_global();
    let spawn_server = netsim::device::MachineBuilder::new()
        .add_ip_iface(
            netsim::iface::IpIfaceBuilder::new()
                .ipv4_addr(server_ip, 0)
                .ipv4_route(Ipv4Route::new(Ipv4Range::global(), None)),
            plug_server,
        ).spawn(&network_handle, move || {
            let bind_addr = SocketAddr::V4(SocketAddrV4::new(server_ip, 0));
            let listener = unwrap!(TcpListener::bind(&bind_addr));
            let server_addr = unwrap!(listener.local_addr());
            let _ = server_addr_tx.send(server_addr);

            let (_client_sock, _client_addr) = unwrap!(listener.accept());
            unwrap!(client_done_rx.recv());
        });

    let spawn_client = netsim::device::MachineBuilder::new()
        .add_ip_iface(
            netsim::iface::IpIfaceBuilder::new()
                .ipv4_addr(Ipv4Addr::random_global(), 0)
                .ipv4_route(Ipv4Route::new(Ipv4Range::global(), None)),
            plug_client,
        ).spawn(&network_handle, move || {
            const SOCKET1_TOKEN: Token = Token(0);
            let evloop = unwrap!(Poll::new());

            let server_addr = unwrap!(server_addr_rx.recv());
            let mut conf = SocketConfig::default();
            conf.keep_alive = Some((1, 1, 3));
            let mut sock1 = unwrap!(TcpSock::connect_with_conf(&server_addr, conf));

            unwrap!(evloop.register(
                &sock1,
                SOCKET1_TOKEN,
                Ready::writable() | Ready::readable(),
                PollOpt::edge(),
            ));

            let mut events = Events::with_capacity(16);
            'event_loop: loop {
                unwrap!(evloop.poll(&mut events, None));
                for ev in events.iter() {
                    match ev.token() {
                        SOCKET1_TOKEN => {
                            // attempt to read so that lost connection would be detected
                            let res: Result<
                                Option<Vec<u8>>,
                                SocketError,
                            > = sock1.read();
                            match res {
                                Err(SocketError::Io(ref e))
                                    if e.kind() == io::ErrorKind::TimedOut =>
                                {
                                    break 'event_loop;
                                }
                                Err(e) => panic!("Expected time out, but got: {:?}", e),
                                Ok(_) => (),
                            }
                        }
                        _ => panic!("Unexpected event"),
                    }
                }
            }

            unwrap!(client_done_tx.send(()));
        });

    unwrap!(evloop.run(spawn_server.join(spawn_client)));
}
