// Copyright 2018 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use libc;
use mio::net::TcpStream;
use std::io;
use std::mem;
use std::os::unix::io::{AsRawFd, RawFd};
use std::time::Duration;
use SocketConfig;

/// Set TCP keep alive options for a given socket, if configured.
pub fn set_keep_alive(stream: &TcpStream, conf: &SocketConfig) -> io::Result<()> {
    if let Some((idle, interval, count)) = conf.keep_alive {
        stream.set_keepalive(Some(Duration::from_secs(u64::from(idle))))?;
        let fd = stream.as_raw_fd();
        set_ip_opt(fd, libc::TCP_KEEPINTVL, interval)?;
        set_ip_opt(fd, libc::TCP_KEEPCNT, count)?;
    }
    Ok(())
}

/// Sets IP level option for a given socketlevel option for a given socket
fn set_ip_opt(sock_fd: RawFd, opt: libc::c_int, val: u32) -> io::Result<()> {
    unsafe {
        let optval: libc::c_int = val as libc::c_int;
        let ret = libc::setsockopt(
            sock_fd,
            libc::IPPROTO_TCP,
            opt,
            &optval as *const _ as *const libc::c_void,
            mem::size_of_val(&optval) as libc::socklen_t,
        );
        if ret != 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
