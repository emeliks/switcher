use crate::compat::{ AsyncStream, AsyncShutdown, Error };
use futures;

impl<T: async_std::io::Read + async_std::io::Write + Unpin + Send> AsyncShutdown for async_tls::client::TlsStream<T> {
	async fn shutdown(&mut self, _how: std::net::Shutdown) -> Result<(), Error> {

		//TlsStream does not have shutdown method, so just we close it
		Ok(futures::io::AsyncWriteExt::close(self).await?)
	}
}

impl<T: async_std::io::Read + async_std::io::Write + Unpin + Send> AsyncStream for async_tls::client::TlsStream<T> {
}

#[inline]
pub async fn tls_client_connection(host: &str, stream: impl async_std::io::Read + async_std::io::Write + Send + Unpin) -> std::io::Result<impl AsyncStream> {
	let connector = async_tls::TlsConnector::default();
	connector.connect(host, stream).await
}
