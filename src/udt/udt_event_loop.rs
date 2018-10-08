use mio::Token;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use udt_extern::{Epoll, UdtSocket};

pub struct EpollWrap<T> {
    epoll: Epoll,
    token_map: HashMap<UdtSocket, Token>,
    tx: T,
}

impl<T: TokenSender> EpollWrap<T> {
    pub fn start_el(tx: T) -> ::Res<Arc<Mutex<Self>>> {
        let mut epoll = Epoll::create()?;

        Ok(Arc::new(Mutex::new(EpollWrap {
            epoll,
            token_map: Default::default(),
            tx,
        })))
    }
}

pub trait TokenSender {}
