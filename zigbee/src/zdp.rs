//zigbee device profile

use deku::{ self, prelude::* };
use serde::{ Deserialize };

//zdo commands are in profile = 0, command set as cluster id

#[derive(Debug)]
pub enum Error {
	Deku(deku::DekuError),
	Json(serde_json::Error),
	NotImplemented,
	BufferTooSmall
}

impl core::fmt::Display for Error {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		match self {
			Self::Deku(e) => e.fmt(f),
			Self::Json(e) => e.fmt(f),
			Self::NotImplemented => write!(f, "Not implemented"),
			Self::BufferTooSmall => write!(f, "Buffer too small"),
		}
	}
}

impl From<deku::DekuError> for Error {
	fn from(e: deku::DekuError) -> Self {
		Error::Deku(e)
	}
}

impl From<serde_json::Error> for Error {
	fn from(e: serde_json::Error) -> Self {
		Error::Json(e)
	}
}

#[derive(Debug, Deserialize, DekuRead, DekuWrite, Clone)]
pub struct ZdoNodeDescReq {
	nwk_addr_of_interest: u16,
}

#[derive(Debug, Deserialize, DekuRead, DekuWrite, Clone)]
pub struct ZdoMatchDescReq {
	nwk_addr_of_interest: u16,
	profile_id: u16,
	num_in_clusters: u8,
	#[deku(count = "num_in_clusters")]
	in_cluster_list: Vec<u8>,
	num_out_clusters: u8,
	#[deku(count = "num_out_clusters")]
	out_cluster_list: Vec<u16>,
}

#[derive(Debug, Deserialize, DekuRead, DekuWrite, Clone)]
pub struct ZdoCabability {
	#[deku(bits = 1)]
	pub allocate_address: bool,
	#[deku(bits = 1)]
	pub security_capability: bool,
	#[deku(bits = 1, pad_bits_before = "2")]
	pub receiver_on_when_idle: bool,
	#[deku(bits = 1)]
	pub power_source: bool,
	#[deku(bits = 1)]
	pub device_type: u8,
	#[deku(bits = 1)]
	pub alternate_pan_coordinator: bool,
}

#[derive(Debug, Deserialize, DekuRead, DekuWrite, Clone)]
pub struct ZdoDeviceAnnce {
	nwk_addr: u16,
	ieee_addr: u64,
	capability: ZdoCabability
}

#[derive(Debug, Deserialize, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8", bytes = "1")]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum BindReqAddr {
	#[deku(id = "0x01")]
	Short {
		dst_addr: u16
	},
	#[deku(id = "0x03")]
	Long {
		dst_addr: u64,
		dst_endp: u8
	},
}

#[derive(Debug, Deserialize, DekuRead, DekuWrite, Clone)]
#[serde(default)]
pub struct ZdoBindReq {
	pub src_address: u64,
	pub src_endp: u8,
	pub cluster_id: u16,
	pub dst_addr: BindReqAddr
}

impl Default for ZdoBindReq {
	fn default() -> Self {
		ZdoBindReq {
			src_address: 0,
			src_endp: 0,
			cluster_id: 0,
			dst_addr: BindReqAddr::Long {
				dst_addr: 0,
				dst_endp: 0
			}
		}
	}
}

#[derive(Debug, Deserialize, DekuWrite, Clone)]
pub struct ZdoRaw {
	#[deku(read_all)]
	data: Vec<u8>
}

#[derive(Debug, Deserialize, DekuWrite, Clone)]
#[serde(tag = "type")]
#[deku(id_type = "u16", bytes = "0")]
pub enum ZdoCommand
{
	#[serde(rename = "ndr")]
	#[deku(id = "0x0002")]
	NodeDescReq(ZdoNodeDescReq),
	#[serde(rename = "mdr")]
	#[deku(id = "0x0006")]
	MatchDescReq(ZdoMatchDescReq),
	#[serde(rename = "annce")]
	#[deku(id = "0x0013")]
	DeviceAnnce(ZdoDeviceAnnce),
	#[serde(rename = "bind")]
	#[deku(id = "0x0021")]
	BindReq(ZdoBindReq),
	#[serde(rename = "raw")]
	#[deku(id = "0x0000")]
	Raw(ZdoRaw)
}

impl ZdoCommand {
	pub fn from_json(value: serde_json::Value) -> Result<Self, Error> {
		Ok(serde_json::from_value::<Self>(value)?)
	}

	pub fn raw(value: Vec<u8>) -> Self {
		Self::Raw(ZdoRaw{data: value})
	}

	pub fn from_buf(buf: &[u8], cluster_id: u16) -> Result<Self, Error> {
		match cluster_id {
			0x0002 => Ok(Self::NodeDescReq(ZdoNodeDescReq::try_from(buf)?)),
			0x0006 => Ok(Self::MatchDescReq(ZdoMatchDescReq::try_from(buf)?)),
			0x0013 => Ok(Self::DeviceAnnce(ZdoDeviceAnnce::try_from(buf)?)),
			0x0021 => Ok(Self::BindReq(ZdoBindReq::try_from(buf)?)),
			_ => Ok(Self::Raw(ZdoRaw{data: buf.to_vec()}))
		}
	}

	pub fn get_cluster_id(&self) -> u16 {
		match self {
			Self::NodeDescReq(_) => 0x0002,
			Self::MatchDescReq(_) => 0x0006,
			Self::DeviceAnnce(_) => 0x0013,
			Self::BindReq(_) => 0x0021,
			Self::Raw(_) => 0x0000
		}
	}
}

#[derive(Debug, DekuWrite, Clone)]
pub struct ZdpFrame {
	#[deku(skip)]
	pub command_no: u16,
	pub sequence_number: u8,
	pub command: ZdoCommand
}

impl ZdpFrame {
	pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
		Ok(DekuContainerWrite::to_bytes(self)?)
	}

	pub fn from_buf(buf: &[u8], cluster_id: u16) -> Result<Self, Error> {
		if buf.len() == 0 {
			return Err(Error::BufferTooSmall);
		}

		Ok(ZdpFrame {
			command_no: cluster_id,
			sequence_number: buf[0],
			command: ZdoCommand::from_buf(&buf[1..], cluster_id)?
		})
	}

	pub fn from_command(command: ZdoCommand) -> Self {
		ZdpFrame{
			command_no: command.get_cluster_id(),
			sequence_number: 0,
			command
		}
	}
}
