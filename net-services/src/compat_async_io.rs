use crate::compat::{ AsyncShutdown, AsyncStream, Error };
use async_io::{ Async };

impl AsyncShutdown for Async<std::net::TcpStream> {
	async fn shutdown(&mut self, how: std::net::Shutdown) -> Result<(), Error> {
		Ok(self.get_ref().shutdown(how)?)
	}
}

impl AsyncStream for Async<std::net::TcpStream> {
}