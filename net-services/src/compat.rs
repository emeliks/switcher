use core::future::Future;

#[derive(Debug)]
pub enum Error {
	IoError(std::io::Error),
	#[cfg(feature="serialport")]
	SerialPort(serialport::Error),
	UnexpectedEof
}

impl core::fmt::Display for Error {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		match self {
			Self::IoError(e) => e.fmt(f),
			#[cfg(feature="serialport")]
			Self::SerialPort(e) => e.fmt(f),
			Self::UnexpectedEof => write!(f, "{self:?}")
		}
	}
}

impl From<std::io::Error> for Error {
	fn from(r: std::io::Error) -> Self {
		Error::IoError(r)
	}
}

#[cfg(feature="serialport")]
impl From<serialport::Error> for Error {
	fn from(se: serialport::Error) -> Self {
		Error::SerialPort(se)
	}
}

pub trait Read {
	fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error>;
	fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<(), Error> {
		while !buf.is_empty() {
			match self.read(buf) {
				Ok(0) => break,
				Ok(n) => buf = &mut buf[n..],
				Err(e) => return Err(e),
			}
		}
		if buf.is_empty() {
			Ok(())
		} else {
			Err(Error::UnexpectedEof)
		}
	}
}

pub trait Write {
	fn write(&mut self, buf: &[u8]) -> Result<usize, Error>;
	fn flush(&mut self) -> Result<(), Error>;

	fn write_all(&mut self, mut buf: &[u8]) -> Result<(), Error> {
		while !buf.is_empty() {
			match self.write(buf) {
				Ok(0) => panic!("write() returned Ok(0)"),
				Ok(n) => buf = &buf[n..],
				Err(e) => return Err(e),
			}
		}
		Ok(())
	}
}

pub trait AsyncRead {
	fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<usize, Error>> + Send;
	fn read_exact(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<(), Error>> + Send;
}

pub trait AsyncWrite {
	fn write(&mut self, buf: &[u8]) -> impl Future<Output = Result<usize, Error>> + Send;
	fn write_all(&mut self, buf: &[u8]) -> impl Future<Output = Result<(), Error>> + Send;
	fn flush(&mut self) -> impl Future<Output = Result<(), Error>> + Send;
}

pub trait AsyncShutdown {
	fn shutdown(&mut self, how: std::net::Shutdown) -> impl Future<Output = Result<(), Error>> + Send;
}

pub trait AsyncStream: AsyncRead + AsyncWrite + AsyncShutdown {
}
