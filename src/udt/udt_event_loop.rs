use maidsafe_utilities::thread::{self, Joiner};
use mio::{Event, Ready, Token};
use std::collections::{HashMap, HashSet};
use std::mem;
use std::sync::{Arc, Mutex, Weak};
use udt_extern::{self, Epoll, EpollEvents, UdtSocket, UDT_EPOLL_ERR, UDT_EPOLL_IN, UDT_EPOLL_OUT};
use SocketError;

pub struct EpollLoop {
    queued_actions: Weak<Mutex<HashSet<QueuedAction>>>,
    _joiner: Joiner,
}

#[derive(Clone)]
pub struct Handle {
    queued_actions: Weak<Mutex<HashSet<QueuedAction>>>,
}

pub trait Notifier {
    fn notify(&self, event: Event);
}

impl EpollLoop {
    pub fn start_event_loop<T: Notifier + Send + 'static>(notifier: T) -> ::Res<EpollLoop> {
        udt_extern::init();

        // Create it just now so that if it fails we can report it immediately
        let epoll = Epoll::create()?;

        let queued_actions = Arc::new(Mutex::new(HashSet::<QueuedAction>::default()));
        let queued_actions_weak = Arc::downgrade(&queued_actions);

        let j = thread::named("UDT Event Loop", move || {
            event_loop_impl(epoll, notifier, queued_actions);
            debug!("Gracefully shut down udt epoll event loop");
        });

        Ok(EpollLoop {
            queued_actions: queued_actions_weak,
            _joiner: j,
        })
    }

    pub fn handle(&self) -> Handle {
        Handle {
            queued_actions: self.queued_actions.clone(),
        }
    }
}

impl Drop for EpollLoop {
    fn drop(&mut self) {
        if let Some(queued_actions) = self.queued_actions.upgrade() {
            unwrap!(queued_actions.lock()).insert(QueuedAction::ShutdownEventLoop);
        }
    }
}

impl Handle {
    pub fn register(&self, sock: UdtSocket, token: Token, interest: Ready) -> ::Res<()> {
        match self.queued_actions.upgrade() {
            Some(queued_actions) => {
                let mut event_set = EpollEvents::empty();
                if interest.is_readable() {
                    event_set.insert(UDT_EPOLL_IN)
                }
                if interest.is_writable() {
                    event_set.insert(UDT_EPOLL_OUT)
                }
                event_set.insert(UDT_EPOLL_ERR);

                let _ = unwrap!(queued_actions.lock()).insert(QueuedAction::Register {
                    sock,
                    token,
                    event_set,
                });
            }
            None => return Err(SocketError::NoUdtEpoll),
        }

        Ok(())
    }

    pub fn deregister(&self, sock: UdtSocket) -> ::Res<()> {
        match self.queued_actions.upgrade() {
            Some(queued_actions) => {
                let _ = unwrap!(queued_actions.lock()).insert(QueuedAction::Deregister { sock });
            }
            None => return Err(SocketError::NoUdtEpoll),
        }

        Ok(())
    }
}

#[cfg_attr(feature = "cargo-clippy", allow(needless_pass_by_value))]
fn event_loop_impl<T: Notifier + Send + 'static>(
    mut epoll: Epoll,
    notifier: T,
    queued_actions: Arc<Mutex<HashSet<QueuedAction>>>,
) {
    let mut token_map: HashMap<UdtSocket, Token> = Default::default();
    loop {
        match epoll.wait(1000, true) {
            Ok((read_ready, write_ready)) => {
                trace!(
                    "Epoll fired with readiness with reads for {} sockets and writes for {}
                       sockets",
                    read_ready.len(),
                    write_ready.len()
                );
                for sock in read_ready {
                    if let Some(token) = token_map.get(&sock) {
                        notifier.notify(Event::new(Ready::readable(), *token));
                    }
                }
                for sock in write_ready {
                    if let Some(token) = token_map.get(&sock) {
                        notifier.notify(Event::new(Ready::writable(), *token));
                    }
                }
            }
            Err(e) => trace!("Broke out of epoll loop: {:?}", e),
        }

        let drained_queued_actions =
            mem::replace(&mut *unwrap!(queued_actions.lock()), Default::default());
        for action in drained_queued_actions {
            match action {
                QueuedAction::Register {
                    sock,
                    token,
                    event_set,
                } => {
                    if let Err(e) = epoll.remove_usock(&sock) {
                        warn!("Error in deregistering UDT socket: {:?}", e);
                    }
                    if let Err(e) = epoll.add_usock(&sock, Some(event_set)) {
                        warn!("Failed to register UDT socket: {:?}", e);
                    }
                    let _ = token_map.insert(sock, token);
                }
                QueuedAction::Deregister { sock } => {
                    if let Err(e) = epoll.remove_usock(&sock) {
                        warn!("Error in deregistering UDT socket: {:?}", e);
                    }
                    let _ = token_map.remove(&sock);
                }
                QueuedAction::ShutdownEventLoop => {
                    // FIXME: get this available
                    // epoll.release();
                    return;
                }
            }
        }
    }
}

#[derive(Eq, PartialEq, Hash)]
enum QueuedAction {
    Register {
        sock: UdtSocket,
        token: Token,
        event_set: EpollEvents,
    },
    Deregister {
        sock: UdtSocket,
    },
    ShutdownEventLoop,
}
