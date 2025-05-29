use crate::frames::{ Error, FrameRead, FrameWrite };
use serialport::{ self, SerialPort, TTYPort };
use std::io::{ Read, Write };

impl From<serialport::Error> for Error {
	fn from(r: serialport::Error) -> Self {
		Error::SerialPort(r)
	}
}

impl FrameRead for TTYPort {
	fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
		if self.bytes_to_read()? == 0 {
			return Err(Error::WouldBlock)
		}

		Ok(Read::read(self, buf)?)
	}
}

impl FrameWrite for TTYPort {
	fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
		if self.bytes_to_write()? != 0 {
			return Err(Error::WouldBlock)
		}

		Ok(Write::write(self, buf)?)
	}

	fn flush(&mut self) -> Result<(), Error> {
		Ok(Write::flush(self)?)
	}
}
