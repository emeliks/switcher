use core::future::Future;

#[derive(Debug)]
pub enum Error {
	Other(String),
	#[cfg(feature="serialport")]
	SerialPort(serialport::Error),
	WouldBlock,
	BufferNotEmpty,
	UnexpectedEof
}

impl core::fmt::Display for Error {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		match self {
			Self::Other(s) => write!(f, "{}", s),
			#[cfg(feature="serialport")]
			Self::SerialPort(s) => s.fmt(f),
			Self::WouldBlock => write!(f, "Would block"),
			Self::BufferNotEmpty => write!(f, "Buffer not empty"),
			Self::UnexpectedEof => write!(f, "Unexpected Eof"),
		}
	}
}

impl Error {
	pub fn need_reset(&self) -> bool {
		match self{
			#[cfg(feature="serialport")]
			Self::SerialPort(_) => true,
			_ => false
		}
	}
}

//traits for async read / write (async tcpstream, etc.)

pub trait AsyncFrameRead {
	fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<usize, Error>> + Send;
}

pub trait AsyncFrameWrite {
	fn write(&mut self, buf: &[u8]) -> impl Future<Output = Result<usize, Error>> + Send;
	fn flush(&mut self) -> impl Future<Output = Result<(), Error>> + Send;
}

//traits for non-blocking non-async read/write (ie. SerialPort)

pub trait FrameRead {
	//read can return 0 in case of there is no data to read
	fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error>;
}

pub trait FrameWrite {
	//write can return 0 in case of there is not place to put outgoing data
	fn write(&mut self, buf: &[u8]) -> Result<usize, Error>;
	fn flush(&mut self) -> Result<(), Error>;
}

#[derive(Default, Debug)]
pub struct FrameBuffer
{
	pub buf: Vec<u8>,
	bytes_to_go: usize
}

impl FrameBuffer {
	pub fn is_empty(&self) -> bool {
		self.bytes_to_go == 0
	}

	pub fn push_frame<F: Frame>(&mut self, frame: &F, params: &F::Params) -> Result<(), Error> {
		if self.buf.len() == 0 {
			frame.as_bytes(params, &mut self.buf)?;
			self.bytes_to_go = self.buf.len();
		}
		else {
			return Err(Error::BufferNotEmpty);
		}

		Ok(())
	}
}

pub trait Frame
{
	type Params: Sync;

	fn get_buffer_len(buf: &mut Vec<u8>, params: &Self::Params) -> Result<usize, Error>;
	fn from_buf(buf: &[u8], params: &Self::Params) -> Result<Self, Error> where Self: Sized;
	fn as_bytes(&self, params: &Self::Params, buf: &mut Vec<u8>) -> Result<(), Error>;

	fn read_buf<R: AsyncFrameRead + Send>(r: &mut R, params: &Self::Params) -> impl Future<Output = Result<Vec<u8>, Error>> + Send {
		async {
			let mut buf = Vec::new();

			loop {
				let desired_diff = Self::get_buffer_len(&mut buf, params)?;

				if desired_diff == 0 {
					break;
				}
				else {
					let len = buf.len();

					buf.resize(len + desired_diff, 0);

					let mut read_buf = &mut buf[len..];
					let mut to_go = desired_diff;

					loop {
						let rr = r.read(&mut read_buf).await?;

						if rr == to_go {
							break;
						}

						if rr == 0 {
							return Err(Error::UnexpectedEof);
						}

						read_buf = &mut read_buf[rr..];
						to_go -= rr;
					}
				}
			}

			Ok(buf)
		}
	}

	fn async_read_frame<R: AsyncFrameRead + Send>(r: &mut R, params: &Self::Params) -> impl Future<Output = Result<Self, Error>> + Send where
		Self: Sized {
		async {
			Ok(Self::from_buf(&Self::read_buf(r, params).await?, params)?)
		}
	}

	fn async_write_frame<W: AsyncFrameWrite + Send>(&self, w: &mut W, params: &Self::Params)  -> impl Future<Output = Result<(), Error>> + Send where
		Self: Sized,
		Self: Sync {
		async {
			let mut vec = Vec::new();
			self.as_bytes(params, &mut vec)?;

			let mut buf = &vec as &[u8];
			let mut to_go = buf.len();

			loop {
				let wl = w.write(&buf).await?;

				if wl == to_go {
					break;
				}

				buf = &buf[wl..];
				to_go -= wl;
			}

			Ok(())
		}
	}

	fn nonblocking_read_frame<R: FrameRead>(r: &mut R, buf: &mut FrameBuffer, params: &Self::Params) -> Result<Option<Self>, Error> where Self: Sized
	{
		if buf.bytes_to_go != 0 {
			let len = buf.buf.len();

			match r.read(&mut buf.buf[len - buf.bytes_to_go..]) {
				Ok(b) => {
					buf.bytes_to_go -= b;
				},
				Err(Error::WouldBlock) => {
				},
				Err(e) => { return Err(e); }
			}
		}

		if buf.bytes_to_go == 0 {
			buf.bytes_to_go = Self::get_buffer_len(&mut buf.buf, params)?;

			if buf.bytes_to_go == 0 {
				let fr = Self::from_buf(&buf.buf, params);

				buf.buf.clear();

				return match fr {
					Ok(f) => Ok(Some(f)),
					Err(e) => Err(e)
				}
			}
			else {
				buf.buf.resize(buf.buf.len() + buf.bytes_to_go, 0);
			}
		}

		Ok(None)
	}

	fn nonblocking_write_frame<W: FrameWrite>(r: &mut W, buf: &mut FrameBuffer) -> Result<bool, Error>
	{
		let buf_len = buf.buf.len();

		match r.write(&buf.buf[buf_len - buf.bytes_to_go..]) {
			Ok(n) => {
				buf.bytes_to_go -= n;

				if buf.bytes_to_go == 0 {
					buf.buf.clear();

					return Ok(true);
				}

			},
			Err(Error::WouldBlock) => {},
			Err(e) => { return Err(e); }
		}


		Ok(false)
	}



}
