pub mod http;
pub mod text;

#[cfg(feature="futures")]
pub mod compat_futures;

#[cfg(feature="async-std")]
pub mod compat_async_std;

#[cfg(feature="async-tls")]
pub mod compat_async_tls;

#[cfg(feature="websocket")]
pub mod websocket;

#[cfg(feature="async-io")]
pub mod compat_async_io;

pub mod compat;

//base network operations

#[cfg(feature="async-tls")]
pub use crate::compat_async_tls::tls_client_connection;

//no tls
#[cfg(not(any(feature="async-tls")))]
pub async fn tls_client_connection(host: &str, stream: impl compat::AsyncStream + Send + Unpin) -> std::io::Result<impl compat::AsyncStream> {
	Ok(stream)
}
