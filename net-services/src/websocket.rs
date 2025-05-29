use core::{ fmt::{ Display, Formatter }, future::Future };
use std::net::{ Shutdown };
use crate::{ http::{ async_http_connection, HttpConnection }, compat::{ self, AsyncShutdown, AsyncRead as Read, AsyncWrite as Write } };
use base64::{engine::general_purpose, Engine as _};

#[derive(Debug)]
pub enum Error {
	IoError(std::io::Error),
	Http(async_http_connection::Error),
	Error(compat::Error)
}

impl Display for Error {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::IoError(e) => e.fmt(f),
			Self::Error(e) => e.fmt(f),
			Self::Http(e) => e.fmt(f),
		}
	}
}

impl From<std::io::Error> for Error {
	fn from(r: std::io::Error) -> Self {
		Error::IoError(r)
	}
}

impl From<compat::Error> for Error {
	fn from(r: compat::Error) -> Self {
		Error::Error(r)
	}
}

impl From<async_http_connection::Error> for Error {
	fn from(r: async_http_connection::Error) -> Self {
		Error::Http(r)
	}
}

//websocket frame

#[derive(Debug)]
pub enum Opcode {
	Continuation,
	Text,
	Binary,
	ConnectionClose,
	Ping,
	Pong,
	Other(u8)
}

impl Opcode {
	pub fn from_u8(b: u8) -> Self {
		match b {
			0 => Self::Continuation,
			1 => Self::Text,
			2 => Self::Binary,
			8 => Self::ConnectionClose,
			9 => Self::Ping,
			10 => Self::Pong,
			o => Self::Other(o),
		}
	}

	pub fn to_u8(&self) -> u8 {
		match self {
			Self::Continuation => 0,
			Self::Text => 1,
			Self::Binary => 2,
			Self::ConnectionClose => 8,
			Self::Ping => 9,
			Self::Pong => 10,
			Self::Other(o) => *o
		}
	}
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct WebsocketFrame {
	fin: bool,
	opcode: Opcode,
	masking_key: [u8; 4],
	payload: Vec<u8>
}

pub async fn async_write_frame<W: Write + Unpin>(w: &mut W, opcode: Opcode, buf: &[u8]) -> Result<(), Error> {
	let payload_len = buf.len();
	let fin = 0b10000000;	//do not split payload
	let mut buf_h = [0; 2];

	//header
	buf_h[0] = fin + opcode.to_u8();
	buf_h[1] = if payload_len > 125 { if payload_len > 0xffff { 127 } else { 126 } } else { payload_len as u8 };

	w.write_all(&buf_h).await?;

	if payload_len > 0 {
		if payload_len > 125 {
			//2 or 6 bytef payload len
			let mut pl = payload_len;
			buf_h[0] = pl as u8;
			pl >>= 8;
			buf_h[1] = pl as u8;
			pl >>= 8;
			w.write_all(&buf_h).await?;

			if pl > 0 {
				for _ in 0..2 {
					buf_h[0] = pl as u8;
					pl >>= 8;
					buf_h[1] = pl as u8;
					pl >>= 8;
					w.write_all(&buf_h).await?;
				}
			}
		}

		//no masking key - server do not mask frame payload
		//payload
		w.write_all(&buf).await?;
	}

	Ok(())
}

pub async fn async_read_frame<R: Read + Unpin>(r: &mut R) -> Result<WebsocketFrame, Error> {
	let mut buf_h = [0; 4];
	r.read_exact(&mut buf_h[0..2]).await?;

	let fin = buf_h[0] & 0b10000000 == 0b10000000;
	let opcode = Opcode::from_u8(buf_h[0] & 0b00001111);
	let is_mask = buf_h[1] & 0b10000000 == 0b10000000;
	let payload_len = async_read_payload_len(r, buf_h[1]).await? as usize;

	if is_mask {
		r.read_exact(&mut buf_h).await?;
	}

	let mut f = WebsocketFrame{
		fin,
		opcode,
		masking_key: buf_h,
		payload: vec![0; payload_len]
	};

	r.read_exact(&mut f.payload).await?;

	if is_mask {
		for i in 0..f.payload.len() {
			f.payload[i] = f.payload[i] ^ f.masking_key[i % 4];
		}
	}

	Ok(f)
}

pub async fn async_read_payload_len<R: Read + Unpin>(r: &mut R, b2: u8) -> Result<u64, Error> {
	let mut payload_len: u64 = (b2 & 0b01111111) as u64;

	if payload_len > 125 {
		let mut buf = vec![0; if payload_len == 126 { 2 } else { 6 }];
		r.read_exact(&mut buf).await?;
		payload_len = 0;

		for b in &buf {
			payload_len <<= 8;
			payload_len += *b as u64;
		}
	}

	Ok(payload_len)
}

pub async fn async_write_handshake<S: Read + Write + Unpin>(c: &mut HttpConnection, s: &mut S) -> Result<(), Error> {
	c.resp.header("Connection", "Upgrade");
	c.resp.header("Upgrade", "websocket");

	if let Some(h) = c.req.get_header("sec-websocket-key") {
		let mut m = sha1_smol::Sha1::new();
		m.update(&format!("{}{}", h, "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes());

		let s = general_purpose::STANDARD.encode(&m.digest().bytes());
		c.resp.header("Sec-WebSocket-Accept", &s.as_str());
	}

	if let Some(h) = c.req.get_header("sec-websocket-protocol") {
		c.resp.header("Sec-WebSocket-Protocol", h);
	}

	async_http_connection::write_http_response(&c.protocol, &c.resp, s, 101, "Switching Protocols").await?;

	Ok(())
}

pub async fn create_ws_connection<S: Read + Write + Unpin + Clone + Send + AsyncShutdown + 'static>(ws_s: &mut S, out_s: &mut S) -> Result<(impl Future<Output = ()>, impl Future<Output = ()>), Error>
{
	//ws -> out
	let mut ws_sr = ws_s.clone();
	let mut out_sw = out_s.clone();

	let a = async move {
		loop {
			match async_read_frame(&mut ws_sr).await {
				Ok(f) => {
					match f.opcode {
						Opcode::Text |
						Opcode::Binary => {
							//write frame payload to out stream
							if let Err(e) = out_sw.write_all(&f.payload).await {
								println!("Ws connection write payload error: {:?}", e);
								break;
							}
						},
						Opcode::ConnectionClose => {
							break;
						},
						Opcode::Ping => {
							//send Pong
							let buf = [0; 0];
							if let Err(e) = async_write_frame(&mut ws_sr, Opcode::Pong, &buf).await {
								println!("Ws connection write ping error: {:?}", e);
								break;
							}
						},
						_ => {}
					}
				},
				Err(e) => {
					println!("Ws connection read error: {:?}", e);
					break;
				}
			}
		}

		//closing both streams
		_ = out_sw.shutdown(Shutdown::Both);
		_ = ws_sr.shutdown(Shutdown::Both);
	};

	let mut out_sr = out_s.clone();
	let mut ws_sw = ws_s.clone();

	//out -> ws
	let b = async move {
		let mut buf = [0; 10];

		loop {
			match out_sr.read(&mut buf).await {
				Ok(n) => {
					//create frame and write to ws
					if let Err(e) = async_write_frame(&mut ws_sw, Opcode::Binary, &buf[0..n]).await {
						println!("Ws out connection write error: {:?}", e);
						break;
					}
				},
				Err(e) => {
					println!("Ws out connection read error: {:?}", e);
					break;
				}
			}
		}

		//closing both streams
		_ = out_sr.shutdown(Shutdown::Both);
		_ = ws_sw.shutdown(Shutdown::Both);
	};

	Ok((a, b))
}
