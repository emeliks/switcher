use crate::frames::{ AsyncFrameRead, AsyncFrameWrite, Error };
use futures::{ AsyncReadExt, AsyncWriteExt };

impl From<futures::io::Error> for Error {
	fn from(r: futures::io::Error) -> Self {
		Error::Other(r.to_string())
	}
}

impl<T: AsyncReadExt + Unpin + Send> AsyncFrameRead for T {
	async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
		Ok(AsyncReadExt::read(self, buf).await?)
	}
}

impl<T: AsyncWriteExt + Unpin + Send> AsyncFrameWrite for T {
	async fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
		Ok(AsyncWriteExt::write(self, buf).await?)
	}

	async fn flush(&mut self) -> Result<(), Error> {
		Ok(AsyncWriteExt::flush(self).await?)
	}
}
