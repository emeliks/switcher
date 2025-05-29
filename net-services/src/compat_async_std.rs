use crate::compat::{ AsyncShutdown, AsyncStream, Error };

impl AsyncShutdown for async_std::net::TcpStream {
	async fn shutdown(&mut self, how: std::net::Shutdown) -> Result<(), Error> {
		Ok(async_std::net::TcpStream::shutdown(self, how)?)
	}
}

impl AsyncStream for async_std::net::TcpStream {
}
