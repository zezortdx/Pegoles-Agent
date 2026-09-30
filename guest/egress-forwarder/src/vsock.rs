//! AF_VSOCK listener and stream (Linux guest only).

use std::fs::File;
use std::io::{self, Read, Write};
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

use crate::transport::{is_host_cid, Acceptor, Transport};

fn cvt(ret: libc::c_int) -> io::Result<libc::c_int> {
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub struct VsockListener(OwnedFd);

impl VsockListener {
    /// Listens on `port` for any CID (the host connects from CID 2).
    pub fn bind(port: u32) -> io::Result<VsockListener> {
        // SAFETY: plain socket(2); the result is checked and owned below.
        let fd = cvt(unsafe {
            libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0)
        })?;
        // SAFETY: `fd` is a fresh descriptor nobody else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        // SAFETY: sockaddr_vm is plain old data; all-zero is a valid value.
        let mut addr: libc::sockaddr_vm = unsafe { mem::zeroed() };
        addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        addr.svm_port = port;
        addr.svm_cid = libc::VMADDR_CID_ANY;
        // SAFETY: `addr` is a valid sockaddr_vm of the stated length.
        cvt(unsafe {
            libc::bind(
                fd.as_raw_fd(),
                (&addr as *const libc::sockaddr_vm).cast(),
                mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
            )
        })?;
        // SAFETY: listen(2) on a bound socket we own.
        cvt(unsafe { libc::listen(fd.as_raw_fd(), 4) })?;
        Ok(VsockListener(fd))
    }
}

impl Acceptor for VsockListener {
    type Conn = VsockStream;

    fn accept(&self) -> io::Result<VsockStream> {
        loop {
            // SAFETY: sockaddr_vm is plain old data; all-zero is valid.
            let mut peer: libc::sockaddr_vm = unsafe { mem::zeroed() };
            let mut len = mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t;
            // SAFETY: `peer` and `len` are valid for accept4 to fill in.
            let fd = cvt(unsafe {
                libc::accept4(
                    self.0.as_raw_fd(),
                    (&mut peer as *mut libc::sockaddr_vm).cast(),
                    &mut len,
                    libc::SOCK_CLOEXEC,
                )
            })?;
            // SAFETY: a fresh descriptor returned by accept4.
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            let from_vsock = len as usize >= mem::size_of::<libc::sockaddr_vm>()
                && peer.svm_family == libc::AF_VSOCK as libc::sa_family_t;
            if from_vsock && is_host_cid(peer.svm_cid) {
                return Ok(VsockStream(File::from(fd)));
            }
            // Not the host: close it and keep waiting.
            log!(
                "refused a vsock peer that is not the host (cid {})",
                peer.svm_cid
            );
        }
    }
}

pub struct VsockStream(File);

impl Read for VsockStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for VsockStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Transport for VsockStream {
    fn try_clone(&self) -> io::Result<Self> {
        self.0.try_clone().map(VsockStream)
    }

    fn shutdown(&self) {
        // SAFETY: shutdown(2) on a descriptor we own; errors are irrelevant.
        unsafe { libc::shutdown(self.0.as_raw_fd(), libc::SHUT_RDWR) };
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        let tv = match timeout {
            Some(d) => libc::timeval {
                tv_sec: d.as_secs().try_into().unwrap_or(libc::time_t::MAX),
                tv_usec: d.subsec_micros().into(),
            },
            None => libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
        };
        // SAFETY: `tv` is a valid timeval of the stated length.
        cvt(unsafe {
            libc::setsockopt(
                self.0.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                (&tv as *const libc::timeval).cast(),
                mem::size_of::<libc::timeval>() as libc::socklen_t,
            )
        })
        .map(|_| ())
    }
}
