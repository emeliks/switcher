use core::{ fmt::{ self, Formatter, Display }, marker::Unpin };
use crate::{ text::{ self, read_line }, http::{ HttpHeaders, HttpResp, HttpConnection, ReqOrResp }, compat::{ self, AsyncRead as Read, AsyncWrite as Write } };
use std::string::FromUtf8Error;

#[derive(Debug)]
pub enum Error {
	IoError(std::io::Error),
	TextError(text::Error),
	Error(compat::Error),
	BadRequest(String),
	OneWriteAllowed,
	FromUtf8Error(FromUtf8Error)
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

impl From<text::Error> for Error {
	fn from(r: text::Error) -> Self {
		Error::TextError(r)
	}
}

impl From<FromUtf8Error> for Error {
	fn from(r: FromUtf8Error) -> Self {
		Error::FromUtf8Error(r)
	}
}

impl Display for Error {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self {
			Self::IoError(e) => e.fmt(f),
			Self::Error(e) => e.fmt(f),
			Self::TextError(e) => e.fmt(f),
			Self::BadRequest(s) => write!(f, "Bad request {}", s),
			Self::OneWriteAllowed => write!(f, "Only one wtrie is allowed by HttpConnection auto_content_length = true"),
			Self::FromUtf8Error(e) => e.fmt(f),
		}
	}
}

pub async fn incoming(s: &mut (impl Read + Write + Unpin)) -> Result<HttpConnection, Error> {
	let l = read_line(s).await?;

	incoming_with_status_line(s, &l).await
}

pub async fn incoming_with_status_line(s: &mut (impl Read + Write + Unpin), l: &str) -> Result<HttpConnection, Error> {
	let v: Vec<&str> = l.split(' ').collect();
	let ln = v.len();

	if ln < 3 {
		return Err(Error::BadRequest(l.to_string()));
	}

	let mut req = HttpHeaders::new();
	let mut resp = HttpHeaders::new();

	//currently we always close connection
	resp.header("Connection", "close");

	read_headers(s, &mut req).await?;

	let c = HttpConnection {
		req,
		resp,
		method: v[0].to_string(),
		path: v[1].to_string(),
		protocol: v[2].to_string(),
		headers_sent: false,
		auto_content_length: true
	};

	Ok(c)
}

async fn read_headers(s: &mut (impl Read + Write + Unpin), h: &mut HttpHeaders) -> Result<(), Error> {
	loop {
		let l = read_line(s).await?;

		if l == "" {
			break;
		}

		let v: Vec<&str> = l.split(": ").collect();

		if v.len() == 2 {
			h.header(v[0], v[1]);
		}
	}

	Ok(())
}

async fn write_http_headers(s: &mut (impl Read + Write + Unpin), h: &HttpHeaders) -> Result<(), Error> {
	for (_, headers) in &h.headers {
		for (k, v) in headers {
			s.write_all(&k.as_bytes()).await?;
			s.write_all(b": ").await?;
			s.write_all(&v.as_bytes()).await?;
			s.write_all(b"\r\n").await?;
		}
	}

	s.write_all(b"\r\n").await?;

	Ok(())
}

pub async fn write_http_response(protocol: &str, h: &HttpHeaders, s: &mut (impl Read + Write + Unpin), status_code: u16, reason: &str) -> Result<(), Error> {
	s.write_all(protocol.as_bytes()).await?;
	s.write_all(format!(" {} {}\r\n", status_code, reason).as_bytes()).await?;
	write_http_headers(s, h).await?;

	Ok(())
}

pub async fn write_content(c: &mut HttpConnection, s: &mut (impl Read + Write + Unpin), content: &[u8]) -> Result<(), Error> {
	if !c.headers_sent {
		if content.len() > 0 && c.auto_content_length {
			c.resp.header("Content-Length", &format!("{}", content.len()));
		}

		write_http_response(&c.protocol, &c.resp, s, 200, "OK").await?;

		c.headers_sent = true;
	}
	else
		if c.auto_content_length {
			return Err(Error::OneWriteAllowed)
		}

	s.write_all(content).await?;

	Ok(())
}

pub async fn flush(c: &mut HttpConnection, s: &mut (impl Read + Write + Unpin)) -> Result<(), Error> {
	let b = [0; 0];
	write_content(c, s, &b).await
}

pub fn header(c: &mut HttpConnection, key: &str, val: &str) -> bool {
	if !c.headers_sent {
		c.resp.header(key, val);

		true
	}
	else {
		false
	}
}

pub async fn get_content_str(h: &HttpHeaders, s: &mut (impl Read + Write + Unpin)) -> Result<String, Error> {
	let l = h.get_content_length();

	if l != 0 {
		let mut v = vec![0; l];
		s.read_exact(&mut v).await?;
		Ok(String::from_utf8(v)?)
	}
	else {
		let mut st = "".to_string();
		
		while let Some(c) = text::read_char(s).await? {
			st.push(c);
		}

		Ok(st)
	}
}

// request

pub async fn write_http_request(method: &str, path: &str, protocol: &str, h: &HttpHeaders, s: &mut (impl Read + Write + Unpin)) -> Result<(), Error> {
	s.write_all(&format!("{} {} {}\r\n", method, path, protocol).as_bytes()).await?;
	write_http_headers(s, h).await?;

	Ok(())
}

pub async fn http_read_req_or_resp(s: &mut (impl Read + Write + Unpin)) -> Result<ReqOrResp, Error> {
	let l = read_line(s).await?;

	if let Ok(r) = http_read_response_with_status_line(s, &l).await {
		return Ok(ReqOrResp::Resp(r));
	}
	else {
		return Ok(ReqOrResp::Req(incoming_with_status_line(s, &l).await?));
	}
}

pub async fn http_read_response(s: &mut (impl Read + Write + Unpin)) -> Result<HttpResp, Error> {
	let l = read_line(s).await?;
	http_read_response_with_status_line(s, &l).await
}

pub async fn http_read_response_with_status_line(s: &mut (impl Read + Write + Unpin), l: &str) -> Result<HttpResp, Error> {
	let mut resp = HttpResp::from_status_line(l)?;

	read_headers(s, &mut resp.headers).await?;

	Ok(resp)
}

pub async fn http_stream_request(method: &str, path: &str, h: &HttpHeaders, data: Option<&[u8]>, s: &mut (impl Read + Write + Unpin)) -> Result<HttpResp, Error> {
	write_http_request(method, path, "HTTP/1.1", &h, s).await?;

	if let Some(d) = data {
		s.write_all(d).await?;
	}

	let resp = http_read_response(s).await?;

	Ok(resp)
}

//http get request

pub async fn http_request(method: &str, host: &str, port: u16, url: &str, headers: Option<HttpHeaders>, data: Option<&[u8]>, s: &mut (impl Read + Write + Unpin)) -> Result<HttpResp, Error> {

	let mut req = match headers {
		Some(h) => h,
		None => HttpHeaders::new()
	};

	req.header("Host", &format!("{}:{}", host, port));

	if let Some(d) = data {
		req.header("Content-Length", &format!("{}", d.len()));
	}

	let resp = http_stream_request(method, url, &req, data, s).await?;

	Ok(resp)
}

pub async fn http_request_str(method: &str, host: &str, port: u16, url: &str, headers: Option<HttpHeaders>, data: Option<&[u8]>, s: &mut (impl Read + Write + Unpin), debug: bool) -> Result<String, Error> {
	if debug {
		println!("{} request: {}:{}{}, headers: {:?}", method, host, port, url, headers);
	}

	let resp = http_request(method, host, port, url, headers, data, s).await?;
	let out_s = get_content_str(&resp.headers, s).await?;

	if debug {
		println!("Response from {}:{}{}: {:?}", host, port, url, resp);
	}

	Ok(out_s)
}
