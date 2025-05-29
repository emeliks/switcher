use core::{ marker::Unpin, fmt::{ self, Display, Formatter } };
use crate::compat::{ self, AsyncRead as Read };

#[derive(Debug)]
pub enum Error {
	InvalidByte(u8),
	InputTruncated,
	SourceError(std::io::Error),
	Error(compat::Error),
}

impl From<std::io::Error> for Error {
	fn from(r: std::io::Error) -> Self {
		Error::SourceError(r)
	}
}

impl From<compat::Error> for Error {
	fn from(r: compat::Error) -> Self {
		Error::Error(r)
	}
}

impl Display for Error {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self {
			Self::InvalidByte(b) => write!(f, "invalid byte 0x{:x}", b),
			Self::InputTruncated => write!(f, "character truncated"),
			Self::SourceError(e) => e.fmt(f),
			Self::Error(e) => e.fmt(f),
		}
	}
}

pub async fn read_line(s: &mut (impl Read + Unpin)) -> Result<String, Error> {
	let mut l = "".to_string();

	loop {
		match read_byte(s).await {
			Ok(Some(b'\r')) => { continue; },
			Ok(Some(b'\n')) => { break; },
			Ok(Some(b)) => {
				l.push(b as char);
			},
			Err(e) => {
				return Err(e);
			},
			Ok(None) => { break; }
		}
	}

	Ok(l)
}

pub async fn read_line_utf8(s: &mut (impl Read + Unpin)) -> Result<String, Error> {
	let mut l = "".to_string();

	loop {
		match read_char(s).await {
			Ok(Some('\r')) => { continue; },
			Ok(Some('\n')) => { break; },
			Ok(Some(c)) => {
				l.push(c);
			},
			Ok(None) => { break },
			Err(e) => {
				return Err(e);
			}
		}
	}

	Ok(l)
}

async fn read_byte(s: &mut (impl Read + Unpin)) -> Result<Option<u8>, Error> {
	let mut buf = [0; 1];

	match s.read(&mut buf).await {
		Ok(1) => Ok(Some(buf[0])),
		Ok(_) => Ok(None),
		Err(e) => Err(Error::Error(e)),
	}
}

pub async fn read_char(s: &mut (impl Read + Unpin)) -> Result<Option<char>, Error> {
	match read_byte(s).await? {
		Some(a) => {
			let a = a as u32;

			let result = if a & 0b1000_0000 == 0 {
				Ok(a)
			} else if a & 0b1110_0000 == 0b1100_0000 {
				let b = continuation(s).await?;
				Ok((a & 0b0001_1111) << 6 | b)
			} else if a & 0b1111_0000 == 0b1110_0000 {
				let b = continuation(s).await?;
				let c = continuation(s).await?;
				Ok((a & 0b0000_1111) << 12 | b << 6 | c)
			} else if a & 0b1111_1000 == 0b1111_0000 {
				let b = continuation(s).await?;
				let c = continuation(s).await?;
				let d = continuation(s).await?;
				Ok((a & 0b0000_0111) << 18 | b << 12 | c << 6 | d)
			} else {
				Err(Error::InvalidByte(a as u8))
			};

			Ok(Some(char::try_from(result?).unwrap()))
		},
		None => {
			Ok(None)
		}
	}
}

async fn continuation(s: &mut (impl Read + Unpin)) -> Result<u32, Error> {
	match read_byte(s).await {
		Ok(Some(byte)) => {
			return if byte & 0b1100_0000 == 0b1000_0000 {
				Ok((byte & 0b0011_1111) as u32)
			} else {
				Err(Error::InvalidByte(byte))
			}
		},
		Ok(None) => Err(Error::InputTruncated),
		Err(e) => Err(e)
	}
}

