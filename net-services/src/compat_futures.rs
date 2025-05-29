use crate::compat::{ AsyncRead, AsyncWrite, Error };
use futures::{ AsyncReadExt, AsyncWriteExt };

impl<T: AsyncReadExt + Unpin + Send> AsyncRead for T {
	async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
		Ok(AsyncReadExt::read(self, buf).await?)
	}

	async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), Error> {
		Ok(AsyncReadExt::read_exact(self, buf).await?)
	}
}

impl<T: AsyncWriteExt + Unpin + Send> AsyncWrite for T {
	async fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
		Ok(AsyncWriteExt::write(self, buf).await?)
	}

	async fn write_all(&mut self, buf: &[u8]) -> Result<(), Error> {
		Ok(AsyncWriteExt::write_all(self, buf).await?)
	}

	async fn flush(&mut self) -> Result<(), Error> {
		Ok(AsyncWriteExt::flush(self).await?)
	}
}
