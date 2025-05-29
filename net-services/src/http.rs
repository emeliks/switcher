pub mod async_http_connection;

use std::collections::HashMap;

#[derive(Debug)]
pub struct HttpHeaders {
	pub headers: HashMap<String, Vec<(String, String)>>,
	content_length: usize
}

impl HttpHeaders {
	pub fn new() -> Self {
		HttpHeaders {
			headers: HashMap::new(),
			content_length: 0
		}
	}

	pub fn header(&mut self, key: &str, val: &str) {
		self.header_str(key, val.to_string());
	}

	pub fn header_str(&mut self, key: &str, val: String) {
		let k = key.to_ascii_lowercase();

		if k == "content-length" {
			self.content_length = match val.parse::<usize>() {
				Ok(l) => l,
				Err(_) => 0
			}
		}

		self.headers.entry(k).or_insert_with(Vec::new).push((key.to_string(), val));
	}

	pub fn get_header(&self, key: &str) -> Option<&str> {
		let k = key.to_ascii_lowercase();

		match self.headers.get(&k) {
			Some(v) => v.get(0).map(|x| &*x.1),
			None => None
		}
	}

	pub fn get_content_length(&self) -> usize {
		self.content_length
	}
}

#[derive(Debug)]
pub struct HttpResp {
	pub http_version: String,
	pub status_code: u16,
	pub reason: String,
	pub headers: HttpHeaders
}

impl HttpResp {
	pub fn from_status_line(l: &str) -> Result<Self, std::io::Error> {
		let v: Vec<&str> = l.split(' ').collect();
		let ln = v.len();

		if ln < 3 || v[0].len() < 5 || &v[0][0..5] != "HTTP/" {
			return Err(std::io::Error::new(std::io::ErrorKind::Other, format!("Bad http response: {}", l)));
		}

		Ok(HttpResp {
			http_version: v[0].to_string(),
			status_code: match v[1].parse::<u16>() { Ok(l) => l, Err(_) => 0 },
			reason: v[2].to_string(),
			headers: HttpHeaders::new(),
		})
	}
}

#[derive(Debug)]
pub struct HttpConnection {
	pub req: HttpHeaders,
	pub resp: HttpHeaders,
	pub method: String,
	pub path: String,
	pub protocol: String,
	headers_sent: bool,
	pub auto_content_length: bool
}

#[derive(Debug)]
pub enum ReqOrResp {
	Req(HttpConnection),
	Resp(HttpResp)
}
