pub mod io {
	#[cfg(feature="async-std")]
	pub use async_std::{ io::{ timeout , Cursor, copy } };
}

pub mod future {
	#[cfg(feature="async-std")]
	pub use async_std::future::timeout;
}

pub mod fs {
	#[cfg(feature="async-std")]
	pub use async_std::{ fs::File };
}

pub mod sync {
	#[cfg(feature="async-std")]
	pub use async_std::{ sync::{ Mutex } };
}

#[cfg(feature="async-std")]
pub mod channel {
	pub use async_std::{ channel::{ Sender, Receiver, unbounded, SendError, RecvError } };
}

#[cfg(feature="async-std")]
pub mod task {
	pub use async_std::{ task::{ self, spawn, sleep } };
}

#[cfg(feature="async-std")]
pub mod net {
	pub use async_std::{ net::{ TcpStream, UdpSocket, TcpListener } };
}
