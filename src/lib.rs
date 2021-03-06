extern crate libc;
extern crate byteorder;

use std::os::unix::io::AsRawFd;
use std::os::unix::io::RawFd;
use std::net::UdpSocket;
use std::net::ToSocketAddrs;
use std::net::SocketAddr;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::io::Cursor;
use std::mem;
use std::time::Duration;
use libc::connect;
use libc::socklen_t;
use libc::sockaddr_in;
use libc::sockaddr_in6;
use libc::in_addr;
use libc::sa_family_t;
use libc::timeval;
use libc::setsockopt;
use libc::SOL_SOCKET;
use libc::AF_INET;
use libc::AF_INET6;
use libc::c_int;
use libc::c_char;
use libc::c_void;
use libc::send;
use libc::recv;
use std::ffi::CString;

use byteorder::{BigEndian, WriteBytesExt};

const SO_RCVTIMEO:c_int = 20;

extern {
	fn inet_pton(family: c_int, src: *const c_char, dst: *mut c_void) -> c_int;
}

pub struct ConnectedSocket<S> {
	sock: S
}

fn to_be(host: u16) -> u16 {
	let mut net = 0u16;

	let len = mem::size_of_val(&net);
	let buf: Vec<u8> = unsafe {
		let ptr:*mut u8 = mem::transmute(&mut net);
		Vec::from_raw_parts(ptr, len, len)
	};
	let mut c = Cursor::new(buf);

	c.write_u16::<BigEndian>(host).unwrap();
	// 'c' is a pointer to 'net' and 'net' is memory-managed,
	// so 'net' is dropped when returning and dropping 'c' would actually
	// double-drop 'net'
	mem::forget(c);

	net
}

impl<S: AsRawFd> AsRawFd for ConnectedSocket<S> {
	fn as_raw_fd(&self) -> RawFd {
		self.sock.as_raw_fd()
	}
}

enum SockaddrIn {
	V4(sockaddr_in),
	V6(sockaddr_in6),
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "nacl",
	target_os = "windows"))]
fn new_sockaddr_in() -> sockaddr_in {
	sockaddr_in {
		sin_family: AF_INET as sa_family_t,
		sin_port:   9,
		sin_zero:   [0; 8],
		sin_addr:   in_addr {
			s_addr: 0
		}
	}
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "nacl",
	target_os = "windows")))]
fn new_sockaddr_in() -> sockaddr_in {
	sockaddr_in {
		sin_len:    mem::size_of::<sockaddr_in>() as u8,
		sin_family: AF_INET as sa_family_t,
		sin_port:   0,
		sin_zero:   [0; 8],
		sin_addr:   in_addr {
			s_addr: 0
		}
	}
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "nacl",
	target_os = "windows"))]
fn new_sockaddr_in6() -> sockaddr_in6 {
	sockaddr_in6 {
		sin6_family:   AF_INET6 as sa_family_t,
		sin6_port:     0,
		sin6_flowinfo: 0,
		sin6_scope_id: 0,
		.. unsafe { mem::zeroed() }
	}
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "nacl",
	target_os = "windows")))]
fn new_sockaddr_in6() -> sockaddr_in6 {
	sockaddr_in6 {
		sin6_len:      mem::size_of::<sockaddr_in6>() as u8,
		sin6_family:   AF_INET6 as sa_family_t,
		sin6_port:     0,
		sin6_flowinfo: 0,
		sin6_scope_id: 0,
		.. unsafe { mem::zeroed() }
	}
}


trait IntoSockaddrIn {
	fn into_sockaddr_in(self) -> Result<SockaddrIn, Error>;
}

impl IntoSockaddrIn for SocketAddr {
	fn into_sockaddr_in(self) -> Result<SockaddrIn, Error> {
		match self {
			SocketAddr::V4(a) => {
				let mut addr = new_sockaddr_in();
				addr.sin_port = to_be(self.port());

				let cstr = CString::new(format!("{}",a.ip())).unwrap();
				let res = unsafe {
					inet_pton(addr.sin_family as c_int,
						cstr.as_ptr() as *const i8,
						mem::transmute(&mut addr.sin_addr))
				};

				if res == 1 {
					Ok(SockaddrIn::V4(addr))
				} else {
					Err(Error::new(ErrorKind::Other,
						"calling inet_pton() for ipv4"))
				}
			},

			SocketAddr::V6(a) => {
				let mut addr = new_sockaddr_in6();
				addr.sin6_port = to_be(self.port());

				let cstr = CString::new(format!("{}",a.ip())).unwrap();
				let res = unsafe {
					inet_pton(addr.sin6_family as c_int,
						cstr.as_ptr() as *const i8,
						mem::transmute(&mut addr.sin6_addr))
				};

				if res > 0 {
					Ok(SockaddrIn::V6(addr))
				} else {
					Err(Error::new(ErrorKind::Other,
						"calling inet_pton() for ipv6"))
				}
			}
		}
	}
}

pub trait Connect {
	fn connect<A: ToSocketAddrs + ?Sized>(self, addr: &A) -> Result<ConnectedSocket<Self>,Error>
		where Self: std::marker::Sized;
}

impl Connect for UdpSocket {
	fn connect<A: ToSocketAddrs + ?Sized>(self, address: &A) -> Result<ConnectedSocket<Self>,Error> {
		let fd = self.as_raw_fd();

		let addr = try!(address.to_socket_addrs()).next();
		if addr.is_none() {
			return Err(Error::new(ErrorKind::InvalidInput,
				"no addresses to connect to"));
		}

		let saddr = try!(addr.unwrap().into_sockaddr_in());

		let res = match saddr {
			SockaddrIn::V4(s) => unsafe {
				let len = mem::size_of_val(&s) as socklen_t;
				let addrp = Box::new(s);
				connect(fd, mem::transmute(&*addrp), len)
			},
			SockaddrIn::V6(s) => unsafe {
				let len = mem::size_of_val(&s) as socklen_t;
				let addrp = Box::new(s);
				connect(fd, mem::transmute(&*addrp), len)
			},
		};
		
		if res == 0 {
			Ok(ConnectedSocket { sock: self })
		} else {	
			Err(Error::new(ErrorKind::Other,
				"error calling connect()"))
		}
	}
}

impl<S: AsRawFd> Read for ConnectedSocket<S> {
	fn read(&mut self, buf: &mut [u8]) -> Result<usize,Error> {
		let flags = 0;
		let ptr = buf.as_mut_ptr() as *mut c_void;

		let len = unsafe {
			recv(self.as_raw_fd(), ptr, buf.len(), flags)
		};

		match len {
			-1 => Err(Error::last_os_error()),
			0 => Err(Error::new(ErrorKind::Other,
				"connection is closed")),
			_ => Ok(len as usize),
		}
	}
}

impl<S: AsRawFd> Write for ConnectedSocket<S> {
	fn write(&mut self, buf: &[u8]) -> Result<usize,Error> {
		let flags = 0;
		let ptr = buf.as_ptr() as *const c_void;

		let res = unsafe {
			send(self.as_raw_fd(), ptr, buf.len(), flags)
		};
		if res >= 0 && (res as usize) == buf.len() {
			Ok(res as usize)
		} else {
			Err(Error::last_os_error())
		}
	}

	fn flush(&mut self) -> Result<(),Error> {
		Ok(())
	}
}

pub trait SetTimeout {
	fn set_timeout(&self, timeout: Duration);
}

impl<S:AsRawFd> SetTimeout for S {
	fn set_timeout(&self, timeout: Duration) {
		let tv = timeval {
			tv_sec: timeout.as_secs() as i64,
			tv_usec: (timeout.subsec_nanos() / 1000) as i64,
		};

		unsafe {
			setsockopt(self.as_raw_fd(), SOL_SOCKET, SO_RCVTIMEO,
				mem::transmute(&tv), mem::size_of_val(&tv) as socklen_t)
		};
	}
}

#[test]
fn connect4_works() {
	let socket1 = UdpSocket::bind("127.0.0.1:34200").unwrap();
	let socket2 = UdpSocket::bind("127.0.0.1:34201").unwrap();
	socket1.connect("127.0.0.1:34200").unwrap();
	socket2.connect("127.0.0.1:34201").unwrap();
}

#[test]
fn sendrecv_works() {
	let socket1 = UdpSocket::bind("127.0.0.1:34200").unwrap();
	let socket2 = UdpSocket::bind("127.0.0.1:34201").unwrap();
	let mut conn1 = socket1.connect("127.0.0.1:34201").unwrap();
	let mut conn2 = socket2.connect("127.0.0.1:34200").unwrap();

	let send1 = [0,1,2,3];
	let send2 = [9,8,7,6];
	conn1.write(&send1).unwrap();
	conn2.write(&send2).unwrap();

	let mut recv1 = [0;4];
	let mut recv2 = [0;4];
	conn1.read(&mut recv1).unwrap();
	conn2.read(&mut recv2).unwrap();

	assert_eq!(send1, recv2);
	assert_eq!(send2, recv1);
}

#[test]
fn sendrecv_respects_packet_borders() {
	let socket1 = UdpSocket::bind("127.0.0.1:34202").unwrap();
	let socket2 = UdpSocket::bind("127.0.0.1:34203").unwrap();
	let mut conn1 = socket1.connect("127.0.0.1:34203").unwrap();
	let mut conn2 = socket2.connect("127.0.0.1:34202").unwrap();

	let send1 = [0,1,2,3];
	let send2 = [9,8,7,6];
	conn1.write(&send1).unwrap();
	conn1.write(&send2).unwrap();

	let mut recv1 = [0;3];
	let mut recv2 = [0;3];
	conn2.read(&mut recv1).unwrap();
	conn2.read(&mut recv2).unwrap();

	assert!(send1[0..3] == recv1[0..3]);
	assert!(send2[0..3] == recv2[0..3]);
}

#[test]
fn connect6_works() {
	let socket1 = UdpSocket::bind("::1:34200").unwrap();
	let socket2 = UdpSocket::bind("::1:34201").unwrap();
	socket1.connect("::1:34200").unwrap();
	socket2.connect("::1:34201").unwrap();
}

#[test]
#[should_panic]
fn detect_invalid_ipv4() {
	let s = UdpSocket::bind("127.0.0.1:34300").unwrap();
	s.connect("255.255.255.255:34200").unwrap();
}

#[test]
#[should_panic]
fn detect_invalid_ipv6() {
	let s = UdpSocket::bind("::1:34300").unwrap();
	s.connect("1200::AB00:1234::2552:7777:1313:34300").unwrap();
}

#[test]
#[should_panic]
fn double_bind() {
	let socket1 = UdpSocket::bind("127.0.0.1:34301").unwrap();
	let socket2 = UdpSocket::bind("127.0.0.1:34301").unwrap();
	drop(socket1);
	drop(socket2);
}
