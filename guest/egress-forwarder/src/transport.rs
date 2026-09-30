//! What the session needs from the host channel, so the logic is testable
//! over a Unix socketpair and runs over AF_VSOCK in the guest.

use std::io::{self, Read, Write};
use std::time::Duration;

/// The vsock CID of the host (`VMADDR_CID_HOST`).
pub const VMADDR_CID_HOST: u32 = 2;

/// Only the host may use the egress channel: any other peer CID (another
/// VM, the hypervisor CID 0, a local loopback CID) is turned away.
pub fn is_host_cid(cid: u32) -> bool {
    cid == VMADDR_CID_HOST
}

pub trait Transport: Read + Write + Send + Sized + 'static {
    /// A second handle to the same connection (reader / writer / control).
    fn try_clone(&self) -> io::Result<Self>;
    /// Shuts both directions down; wakes blocked readers and writers.
    fn shutdown(&self);
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
}

pub trait Acceptor {
    type Conn: Transport;
    fn accept(&self) -> io::Result<Self::Conn>;
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::net::Shutdown;
    use std::os::unix::net::{UnixListener, UnixStream};

    impl Transport for UnixStream {
        fn try_clone(&self) -> io::Result<Self> {
            UnixStream::try_clone(self)
        }
        fn shutdown(&self) {
            let _ = UnixStream::shutdown(self, Shutdown::Both);
        }
        fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            UnixStream::set_read_timeout(self, timeout)
        }
    }

    impl Acceptor for UnixListener {
        type Conn = UnixStream;
        fn accept(&self) -> io::Result<UnixStream> {
            UnixListener::accept(self).map(|(s, _)| s)
        }
    }
}
