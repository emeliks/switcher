use mqtt::mqtt_packet;
use net_services::{ websocket, compat };
use crate::{ switcher::{ SwMessage, System, services::{ self } } };
use tokenizer;
use crate::compat::channel::{ SendError, RecvError };
use core::{ convert::{ Into, TryInto, Infallible }, ops::{ Add, Neg, Sub, BitAnd, Div, BitOr, Mul }, cmp::{ PartialOrd, Ordering } };
use std::{ str::FromStr, num::ParseIntError, collections::HashMap, fmt::{ Display, Formatter } };
use serde::{ Deserialize };
use net_services::http::async_http_connection;
use frames;
use zigbee::zcl::{ self, AttributeValue };
use zigbee::tuya::DpDataValue;
use crate::switcher::services::zgate;

#[derive(Debug)]
pub enum Error {
	String(String),
	Str(&'static str),
	IoError(std::io::Error),
	SendError(SendError<SwMessage>),
	RecvError(RecvError),
	ParseIntError(ParseIntError),
	SerdeJson(serde_json::Error),
	Tokenizer(tokenizer::Error),
	MqttPacket(mqtt_packet::Error),
	Infallible(Infallible),
	WebsocketFrame(websocket::Error),
	Http(async_http_connection::Error),
	Error(compat::Error),
	TextError(net_services::text::Error),
	Frames(frames::Error),
	ZGate(zgate::Error),
	Zcl(zcl::Error),
	Format(core::fmt::Error)
}

impl Display for Error {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::String(s) => write!(f, "{}", s),
			Self::Str(s) => write!(f, "{}", s),
			Self::IoError(e) => e.fmt(f),
			Self::SendError(e) => e.fmt(f),
			Self::RecvError(e) => e.fmt(f),
			Self::ParseIntError(e) => e.fmt(f),
			Self::SerdeJson(e) => e.fmt(f),
			Self::Tokenizer(t) => t.fmt(f),
			Self::MqttPacket(p) => p.fmt(f),
			Self::Infallible(i) => i.fmt(f),
			Self::WebsocketFrame(w) => w.fmt(f),
			Self::Http(h) => h.fmt(f),
			Self::Error(e) => e.fmt(f),
			Self::TextError(e) => e.fmt(f),
			Self::Frames(e) => e.fmt(f),
			Self::ZGate(e) => e.fmt(f),
			Self::Zcl(e) => e.fmt(f),
			Self::Format(e) => e.fmt(f)
		}
	}
}

impl From<core::fmt::Error>for Error {
	fn from(e: core::fmt::Error) -> Self {
		Error::Format(e)
	}
}

impl From<zcl::Error>for Error {
	fn from(e: zcl::Error) -> Self {
		Error::Zcl(e)
	}
}

impl From<net_services::text::Error> for Error {
	fn from(e: net_services::text::Error) -> Self {
		Error::TextError(e)
	}
}

impl From<frames::Error> for Error {
	fn from(e: frames::Error) -> Self {
		Error::Frames(e)
	}
}
impl From<zgate::Error> for Error {
	fn from(e: zgate::Error) -> Self {
		Error::ZGate(e)
	}
}

impl From<compat::Error> for Error {
	fn from(e: compat::Error) -> Self {
		Error::Error(e)
	}
}

impl From<websocket::Error> for Error {
	fn from(w: websocket::Error) -> Self {
		Error::WebsocketFrame(w)
	}
}

impl From<async_http_connection::Error> for Error {
	fn from(h: async_http_connection::Error) -> Self {
		Error::Http(h)
	}
}

impl From<mqtt_packet::Error> for Error {
	fn from(r: mqtt_packet::Error) -> Self {
		Error::MqttPacket(r)
	}
}

impl From<std::io::Error> for Error {
	fn from(r: std::io::Error) -> Self {
		Error::IoError(r)
	}
}

impl From<tokenizer::Error> for Error {
	fn from(r: tokenizer::Error) -> Self {
		Error::Tokenizer(r)
	}
}

impl From<SendError<SwMessage>> for Error {
	fn from(r: SendError<SwMessage>) -> Self {
		Error::SendError(r)
	}
}

impl From<RecvError> for Error {
	fn from(r: RecvError) -> Self {
		Error::RecvError(r)
	}
}

impl From<ParseIntError> for Error {
	fn from(r: ParseIntError) -> Self {
		Error::ParseIntError(r)
	}
}

impl From<serde_json::Error> for Error {
	fn from(r: serde_json::Error) -> Self {
		Error::SerdeJson(r)
	}
}

impl From<&str> for Error {
	fn from(r: &str) -> Self {
		Error::String(r.to_string())
	}
}

impl From<Infallible> for Error {
	fn from(i: Infallible) -> Self {
		Error::Infallible(i)
	}
}

impl From<Error> for std::io::Error {
	fn from(e: Error) -> Self {
		std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
	}
}

#[derive(Clone, PartialEq, Deserialize, Default, Debug)]
pub struct RGBW {
	pub r: u8,
	pub g: u8,
	pub b: u8,
	pub w: u8
}

#[derive(Clone, PartialEq, Deserialize, Debug)]
#[serde(untagged)]
pub enum SwValue {
	None,
	Bool(bool),
	String(String),
	Numeric(i32),
	Float(f32),
	Map(HashMap<String, SwValue>),
	Vec(Vec<SwValue>)
}

impl PartialOrd for SwValue {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		match self {
			Self::Bool(b) => { match TryInto::<bool>::try_into(other.clone()) { Ok(ob) => Some(b.cmp(&ob)), Err(_) => None } },
			Self::String(s) => { match TryInto::<String>::try_into(other.clone()) { Ok(os) => Some(s.cmp(&os)), Err(_) => None } },
			Self::Numeric(n) => {
				match other {
					Self::Float(of) => (*n as f32).partial_cmp(of),
					Self::Numeric(on) => n.partial_cmp(on),
					_ => None
				}
			},
			Self::Float(f) => { match TryInto::<f32>::try_into(other.clone()) { Ok(of) => Some(f.partial_cmp(&of).unwrap_or(Ordering::Equal)), Err(_) => None } },
			_ => None
		}
	}
}

impl SwValue {
	pub fn into_string(self) -> Result<String, Error> {
		Ok(TryInto::<String>::try_into(self)?)
	}

	pub fn into_numeric(self) -> Result<i32, Error> {
		Ok(TryInto::<i32>::try_into(self)?)
	}

	pub fn into_float(self) -> Result<f32, Error> {
		Ok(TryInto::<f32>::try_into(self)?)
	}

	pub fn into_bool(self) -> Result<bool, Error> {
		Ok(TryInto::<bool>::try_into(self)?)
	}

	pub fn into_json(self) -> Result<serde_json::value::Value, Error> {
		match self {
			Self::None => Ok(serde_json::value::Value::Null),
			Self::Bool(b) => Ok(serde_json::value::Value::Bool(b)),
			Self::String(s) => Ok(serde_json::value::Value::String(s)),
			Self::Numeric(i) => Ok(serde_json::value::Value::Number(serde_json::Number::from(i))),
			Self::Float(f) => Ok(match serde_json::Number::from_f64(f as f64) {
				Some(n) => serde_json::value::Value::Number(n),
				None => serde_json::value::Value::Null
			}),
			Self::Map(m) => {
				let mut jm = serde_json::Map::with_capacity(m.len());

				for (k, v) in m {
					jm.insert(k, v.into_json()?);
				}

				Ok(serde_json::value::Value::Object(jm))
			},
			Self::Vec(vec) => {
				//non optimal, but no empty Vec constructor in serde context
				let mut jv = serde_json::json!([]);

				if let Some(ja) = jv.as_array_mut() {
					for v in vec {
						ja.push(v.into_json()?);
					}
				}

				Ok(jv)
			}
		}
		//
	}
}

impl Add for SwValue {
	type Output = Self;

	fn add(self, other: Self) -> Self {
		match other {
			Self::Numeric(on) => {
				match self {
					Self::Numeric(n) => { Self::Numeric(n + on) },
					Self::Float(f) => { Self::Float(f + on as f32) },
					o => { Self::String(format!("{}{}", o, on)) },
				}
			},
			Self::Float(of) => {
				match self {
					Self::Numeric(n) => { Self::Float(n as f32 + of) },
					Self::Float(f) => { Self::Float(f + of) },
					o => { Self::String(format!("{}{}", o, of)) },
				}
			},
			o => {
				Self::String(format!("{}{}", self, o))
			}
		}
	}
}

impl Neg for SwValue {
	type Output = Self;

	fn neg(self) -> Self {
		match self {
			Self::Numeric(n) => Self::Numeric(-n),
			Self::Float(f) => Self::Float(-f),
			_ => Self::None,
		}
	}
}

//todo into float if one of arguments float

impl Sub for SwValue {
	type Output = Self;

	fn sub(self, other: Self) -> Self {
		match other {
			Self::Numeric(on) => {
				match self {
					Self::Numeric(n) => { Self::Numeric(n - on) },
					Self::Float(f) => { Self::Float(f - on as f32) },
					_ => Self::None,
				}
			},
			Self::Float(of) => {
				match self {
					Self::Numeric(n) => { Self::Float(n as f32 - of) },
					Self::Float(f) => { Self::Float(f - of) },
					_ => Self::None,
				}
			},
			_ => Self::None,
		}
	}
}

impl Div for SwValue {
	type Output = Self;

	fn div(self, other: Self) -> Self {
		match other {
			Self::Numeric(on) => {
				match self {
					Self::Numeric(n) => { Self::Numeric(n / on) },
					Self::Float(f) => { Self::Float(f / on as f32) },
					_ => Self::None,
				}
			},
			Self::Float(of) => {
				match self {
					Self::Numeric(n) => { Self::Float(n as f32 / of) },
					Self::Float(f) => { Self::Float(f / of) },
					_ => Self::None,
				}
			},
			_ => Self::None,
		}
	}
}

impl Mul for SwValue {
	type Output = Self;

	fn mul(self, other: Self) -> Self {
		match other {
			Self::Numeric(on) => {
				match self {
					Self::Numeric(n) => { Self::Numeric(n * on) },
					Self::Float(f) => { Self::Float(f * on as f32) },
					_ => Self::None,
				}
			},
			Self::Float(of) => {
				match self {
					Self::Numeric(n) => { Self::Float(n as f32 * of) },
					Self::Float(f) => { Self::Float(f * of) },
					_ => Self::None,
				}
			},
			_ => Self::None,
		}
	}
}

impl BitAnd for SwValue {
	type Output = Self;

	fn bitand(self, other: Self) -> Self {
		match self {
			Self::Numeric(n) => { match TryInto::<i32>::try_into(other) { Ok(on) => Self::Numeric(n & on), Err(_) => Self::None } },
			_ => Self::None,
		}
	}
}

impl BitOr for SwValue {
	type Output = Self;

	fn bitor(self, other: Self) -> Self {
		match self {
			Self::Numeric(n) => { match TryInto::<i32>::try_into(other) { Ok(on) => Self::Numeric(n | on), Err(_) => Self::None } },
			_ => Self::None,
		}
	}
}

impl TryInto<bool> for SwValue {
	type Error = Error;

	fn try_into(self) -> Result<bool, Self::Error> {
		match self {
			Self::Bool(b) => Ok(b),
			Self::Numeric(n) => Ok(n == 1),
			Self::String(s) => Ok(s == "true"),
			_ => Err(Error::String(format!("required bool, found {:?}", self)))
		}
	}
}

impl TryInto<String> for SwValue {
	type Error = Error;

	fn try_into(self) -> Result<String, Self::Error> {
		match self {
			Self::String(s) => Ok(s),
			Self::Numeric(n) => Ok(format!("{}", n)),
			Self::Float(f) => Ok(format!("{}", f)),
			_ => Err(Error::String(format!("required String, found {:?}", self)))
		}
	}
}

impl TryInto<i32> for SwValue {
	type Error = Error;

	fn try_into(self) -> Result<i32, Self::Error> {
		match self {
			Self::Numeric(n) => Ok(n),
			Self::String(s) => Ok(s.parse::<i32>()?),
			_ => Err(Error::String(format!("required i32, found {:?}", self)))
		}
	}
}

impl TryInto<f32> for SwValue {
	type Error = Error;

	fn try_into(self) -> Result<f32, Self::Error> {
		match self {
			Self::Numeric(n) => Ok(n as f32),
			Self::Float(f) => Ok(f),
			_ => Err(Error::String(format!("required f32, found {:?}", self)))
		}
	}
}

impl From<bool> for SwValue {
	fn from(value: bool) -> Self {
		SwValue::Bool(value)
	}
}

impl From<i32> for SwValue {
	fn from(value: i32) -> Self {
		SwValue::Numeric(value)
	}
}

impl From<f32> for SwValue {
	fn from(value: f32) -> Self {
		SwValue::Float(value)
	}
}

impl From<String> for SwValue {
	fn from(value: String) -> Self {
		SwValue::String(value)
	}
}

impl From<AttributeValue> for SwValue {
	fn from(value: AttributeValue) -> Self {
		match value {
			AttributeValue::Data8{ val } => Self::Numeric(val as i32),
			AttributeValue::Data16{ val } => Self::Numeric(val as i32),
			AttributeValue::Data24{ val } => Self::Numeric(val as i32),
			AttributeValue::Data32{ val } => Self::Numeric(val as i32),
			AttributeValue::Map8{ val } => Self::Numeric(val as i32),
			AttributeValue::Map16{ val } => Self::Numeric(val as i32),
			AttributeValue::Map24{ val } => Self::Numeric(val as i32),
			AttributeValue::Map32{ val } => Self::Numeric(val as i32),
			AttributeValue::Bool { val } => Self::Bool(val),
			AttributeValue::Uint8 { val } => Self::Numeric(val as i32),
			AttributeValue::Uint16 { val } => Self::Numeric(val as i32),
			AttributeValue::Uint24 { val } => Self::Numeric(val as i32),
			AttributeValue::Uint32 { val } => Self::Numeric(val as i32),
			AttributeValue::Int8 { val } => Self::Numeric(val as i32),
			AttributeValue::Int16 { val } => Self::Numeric(val as i32),
			AttributeValue::Int24 { val } => Self::Numeric(val as i32),
			AttributeValue::Int32 { val } => Self::Numeric(val as i32),
			AttributeValue::Enum8 { val } => Self::Numeric(val as i32),
			AttributeValue::Enum16 { val } => Self::Numeric(val as i32),
			AttributeValue::Semi { val } => Self::Float(val as f32),
			AttributeValue::Single { val } => Self::Float(val as f32),
			AttributeValue::String { val, .. } => Self::String(String::from_utf8_lossy(&val).to_string()),
			AttributeValue::String16 { val, .. } => Self::String(String::from_utf8_lossy(&val).to_string()),
			_ => Self::None
		}
	}
}

impl SwValue {
	pub fn to_tav(self, t: u8) -> Result<DpDataValue, Error> {
		match t {
			0x00 => Ok(DpDataValue::Raw{count: 1, value: vec![self.into_numeric()? as u8]}),
			0x01 => Ok(DpDataValue::Boolean{count: 1, value: self.into_numeric()? == 1}),
			0x02 => Ok(DpDataValue::Value{count: 4, value: self.into_numeric()? as u32}),
			0x03 => {let s = self.into_string()?.as_bytes().to_vec(); Ok(DpDataValue::String{count: s.len() as u16, value: s})},
			0x04 => Ok(DpDataValue::Enum{count: 1, value: self.into_numeric()? as u8}),
			0x05 => Ok(DpDataValue::Bitmap{count: 1, value: vec![self.into_numeric()? as u8]}),
			_ => Err(Error::String(format!("Bad to_tav type: {}", t)))
		}
	}

	pub fn to_av(self, t: u8) -> AttributeValue {
		let n;

		if let Ok(nv) = self.into_numeric() {
			n = nv;
		}
		else {
			return AttributeValue::Unk;
		}

		match t {
			0xff => AttributeValue::Unk,
			0x00 => AttributeValue::Nodata,
			0x08 => AttributeValue::Data8{ val: n as u8 },
			0x09 => AttributeValue::Data16{ val: n as u16 },
			0x0a => AttributeValue::Data24{ val: n as u32 },
			0x0b => AttributeValue::Data32{ val: n as u32 },
			0x0c => AttributeValue::Data40{ val: n as u64 },
			0x0d => AttributeValue::Data48{ val: n as u64 },
			0x0e => AttributeValue::Data56{ val: n as u64 },
			0x0f => AttributeValue::Data64{ val: n as u64 },
			0x10 => AttributeValue::Bool{ val: n != 0 },
			0x18 => AttributeValue::Map8{ val: n as u8 },
			0x19 => AttributeValue::Map16{ val: n as u16 },
			0x1a => AttributeValue::Map24{ val: n as u32 },
			0x1b => AttributeValue::Map32{ val: n as u32 },
			0x1c => AttributeValue::Map40{ val: n as u64 },
			0x1d => AttributeValue::Map48{ val: n as u64 },
			0x1e => AttributeValue::Map56{ val: n as u64 },
			0x1f => AttributeValue::Map64{ val: n as u64 },
			0x20 => AttributeValue::Uint8{ val: n as u8 },
			0x21 => AttributeValue::Uint16{ val: n as u16 },
			0x22 => AttributeValue::Uint24{ val: n as u32 },
			0x23 => AttributeValue::Uint32{ val: n as u32 },
			0x24 => AttributeValue::Uint40{ val: n as u64 },
			0x25 => AttributeValue::Uint48{ val: n as u64 },
			0x26 => AttributeValue::Uint56{ val: n as u64 },
			0x27 => AttributeValue::Uint64{ val: n as u64 },
			0x28 => AttributeValue::Int8{ val: n as i8 },
			0x29 => AttributeValue::Int16{ val: n as i16 },
			0x2a => AttributeValue::Int24{ val: n as i32 },
			0x2b => AttributeValue::Int32{ val: n as i32 },
			0x2c => AttributeValue::Int40{ val: n as i64 },
			0x2d => AttributeValue::Int48{ val: n as i64 },
			0x2e => AttributeValue::Int56{ val: n as i64 },
			0x2f => AttributeValue::Int64{ val: n as i64 },
			0x30 => AttributeValue::Enum8{ val: n as u8 },
			0x31 => AttributeValue::Enum16{ val: n as u16 },
			0x38 => AttributeValue::Semi{ val: n as f32 },
			0x39 => AttributeValue::Single{ val: n as f32 },
			0x3a => AttributeValue::Double{ val: n as f64 },
			_ => AttributeValue::Unk
		}
	}
}

//no '"' - implementation used in to_string() method

impl Display for SwValue {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::None => write!(f, "none"),
			Self::Bool(b) => write!(f, "{}", b),
			Self::String(s) => write!(f, "{}", s),
			Self::Numeric(n) => write!(f, "{}", n),
			Self::Float(n) => write!(f, "{}", n),
			Self::Map{ .. } => write!(f, "map"),
			Self::Vec{ .. } => write!(f, "vec"),
		}
	}
}

impl FromStr for SwValue {
	type Err = Error;
	fn from_str(input: &str) -> Result<SwValue, Self::Err> {
		match input {
			"none" => Ok(Self::None),
			"true" => Ok(Self::Bool(true)),
			"false" => Ok(Self::Bool(false)),
			_ => {
				let mut v = Self::None;
				let l = input.len();

				if l > 1 {
					if &input[..1] == "\"" && &input[l - 1..] == "\"" {
						v = Self::String((&input[1..l - 1]).to_string());
					}
				}

				if let Self::None = v {
					if let Ok(n) = input.parse::<i32>() {
						v = Self::Numeric(n);
					}
					else {
						if let Ok(n) = input.parse::<f32>() {
							v = Self::Float(n);
						}
					}
				}

				Ok(v)
			}
		}
	}
}

impl FromStr for RGBW {
	type Err = Error;
	fn from_str(input: &str) -> Result<RGBW, Self::Err> {
		let v = decode_hex(input)?;

		Ok(RGBW{r: *v.get(0).unwrap(), g: *v.get(1).unwrap(), b: *v.get(2).unwrap(), w: *v.get(3).unwrap()})
	}
}

impl RGBW {
	pub fn to_string(&self) -> String {
		format!("{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.w)
	}
}

pub fn decode_hex(s: &str) -> Result<Vec<u8>, ParseIntError > {
	(0..s.len())
		.step_by(2)
		.map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.into()))
		.collect()
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub enum SwService {
	#[serde(rename = "db")]
	Db,
	#[serde(rename = "hue")]
	Hue(services::hue_bridge::HueBridge),
	#[serde(rename = "switcher")]
	HttpSwitcher(services::http_switcher_server::HttpSwitcher),
}

impl SwService {
	pub async fn run(&self, system: &mut System) -> Result<(), Error> {
		match self {
			SwService::Hue(h) => { h.run(system).await },
			SwService::HttpSwitcher(s) => { s.run(system).await },
			_ => Ok(())
		}
	}
}
