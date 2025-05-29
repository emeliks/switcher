//zigbee cluster library

use deku::{ self, prelude::* };
use std::{ convert::{ TryFrom } };
use serde::{ Serialize, Deserialize };

#[derive(Debug)]
pub enum Error {
	Deku(deku::DekuError),
	Json(serde_json::Error),
	BufferTooSmall(&'static str),
	NotImplemented
}

impl core::fmt::Display for Error {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		match self {
			Self::Deku(e) => e.fmt(f),
			Self::Json(e) => e.fmt(f),
			Self::BufferTooSmall(s) => write!(f, "Buffer too small: {}", s),
			Self::NotImplemented => write!(f, "Not implemented")
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

#[derive(Debug, Serialize, Deserialize, DekuRead, DekuWrite, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
#[deku(id_type = "u8")]
pub enum AttributeValue {
	#[deku(id = "0xff")]
	Unk,
	#[deku(id = "0x00")]
	Nodata,
	#[deku(id = "0x08")]
	Data8 {
		val: u8
	},
	#[deku(id = "0x09")]
	Data16 {
		val: u16
	},
	#[deku(id = "0x0a")]
	Data24 {
		#[deku(bits = 24)]
		val: u32
	},
	#[deku(id = "0x0b")]
	Data32 {
		val: u32
	},
	#[deku(id = "0x0c")]
	Data40 {
		#[deku(bits = 40)]
		val: u64
	},
	#[deku(id = "0x0d")]
	Data48 {
		#[deku(bits = 48)]
		val: u64
	},
	#[deku(id = "0x0e")]
	Data56 {
		#[deku(bits = 56)]
		val: u64
	},
	#[deku(id = "0x0f")]
	Data64 {
		val: u64
	},
	#[deku(id = "0x10")]
	Bool {
		val: bool
	},
	#[deku(id = "0x18")]
	Map8 {
		val: u8
	},
	#[deku(id = "0x19")]
	Map16 {
		val: u16
	},
	#[deku(id = "0x1a")]
	Map24 {
		#[deku(bits = 24)]
		val: u32
	},
	#[deku(id = "0x1b")]
	Map32 {
		val: u32
	},
	#[deku(id = "0x1c")]
	Map40 {
		#[deku(bits = 40)]
		val: u64
	},
	#[deku(id = "0x1d")]
	Map48 {
		#[deku(bits = 48)]
		val: u64
	},
	#[deku(id = "0x1e")]
	Map56 {
		#[deku(bits = 56)]
		val: u64
	},
	#[deku(id = "0x1f")]
	Map64 {
		val: u64
	},
	#[deku(id = "0x20")]
	Uint8 {
		val: u8
	},
	#[deku(id = "0x21")]
	Uint16 {
		val: u16
	},
	#[deku(id = "0x22")]
	Uint24 {
		#[deku(bits = 24)]
		val: u32
	},
	#[deku(id = "0x23")]
	Uint32 {
		val: u32
	},
	#[deku(id = "0x24")]
	Uint40 {
		#[deku(bits = 40)]
		val: u64
	},
	#[deku(id = "0x25")]
	Uint48 {
		#[deku(bits = 48)]
		val: u64
	},
	#[deku(id = "0x26")]
	Uint56 {
		#[deku(bits = 56)]
		val: u64
	},
	#[deku(id = "0x27")]
	Uint64 {
		val: u64
	},
	#[deku(id = "0x28")]
	Int8 {
		val: i8
	},
	#[deku(id = "0x29")]
	Int16 {
		val: i16
	},
	//check negative values for non standard int length
	#[deku(id = "0x2a")]
	Int24 {
		#[deku(bits = 24)]
		val: i32
	},
	#[deku(id = "0x2b")]
	Int32 {
		val: i32
	},
	#[deku(id = "0x2c")]
	Int40 {
		#[deku(bits = 40)]
		val: i64
	},
	#[deku(id = "0x2d")]
	Int48 {
		#[deku(bits = 48)]
		val: i64
	},
	#[deku(id = "0x2e")]
	Int56 {
		#[deku(bits = 56)]
		val: i64
	},
	#[deku(id = "0x2f")]
	Int64 {
		val: i64
	},
	#[deku(id = "0x30")]
	Enum8 {
		val: u8
	},
	#[deku(id = "0x31")]
	Enum16 {
		val: u16
	},
	#[deku(id = "0x38")]
	Semi {
		#[deku(bits = 16)]
		val: f32
	},
	#[deku(id = "0x39")]
	Single {
		val: f32
	},
	#[deku(id = "0x3a")]
	Double {
		val: f64
	},
	#[deku(id = "0x41")]
	Octstr {
		count: u8,
		#[deku(count = "count")]
		val: Vec<u8>
	},
	#[deku(id = "0x42")]
	String {
		count: u8,
		#[deku(count = "count")]
		val: Vec<u8>
	},
	#[deku(id = "0x43")]
	Octstr16 {
		count: u16,
		#[deku(count = "count")]
		val: Vec<u8>
	},
	#[deku(id = "0x44")]
	String16 {
		count: u16,
		#[deku(count = "count")]
		val: Vec<u8>
	},
	//#[deku(id = "0x48")]
	//Array,
	//#[deku(id = "0x4c")]
	//Struct,
	//#[deku(id = "0x50")]
	//Set,
	//#[deku(id = "0x51")]
	//Bag,
	#[deku(id = "0xe0")]
	ToD {
		h: u8,
		m: u8,
		s: u8,
		cs: u8
	},
	#[deku(id = "0xe1")]
	Date {
		y: u8,
		m: u8,
		d: u8,
		dow: u8
	},
	#[deku(id = "0xe2")]
	UTC {
		val: u32
	},
	#[deku(id = "0xe8")]
	ClusterId {
		val: u16
	},
	#[deku(id = "0xe9")]
	AttribId {
		val: u16
	},
	#[deku(id = "0xea")]
	BacOID {
		val: u32
	},
	#[deku(id = "0xf0")]
	Eui64 {
		val: [u8; 8]
	},
	#[deku(id = "0xf1")]
	Key128 {
		val: [u8; 16]
	},
}

#[derive(Debug, Serialize, Deserialize, DekuRead, DekuWrite, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
#[deku(ctx = "data_type: u8", id = "data_type")]
pub enum AttributeChangeValue {
	#[deku(id = "0x20")]
	Uint8 {
		val: u8
	},
	#[deku(id = "0x21")]
	Uint16 {
		val: u16
	},
	#[deku(id = "0x22")]
	Uint24 {
		#[deku(bits = 24)]
		val: u32
	},
	#[deku(id = "0x23")]
	Uint32 {
		val: u32
	},
	#[deku(id = "0x24")]
	Uint40 {
		#[deku(bits = 40)]
		val: u64
	},
	#[deku(id = "0x25")]
	Uint48 {
		#[deku(bits = 48)]
		val: u64
	},
	#[deku(id = "0x26")]
	Uint56 {
		#[deku(bits = 56)]
		val: u64
	},
	#[deku(id = "0x27")]
	Uint64 {
		val: u64
	},
	#[deku(id = "0x28")]
	Int8 {
		val: i8
	},
	#[deku(id = "0x29")]
	Int16 {
		val: i16
	},
	//check negative values for non standard int length
	#[deku(id = "0x2a")]
	Int24 {
		#[deku(bits = 24)]
		val: i32
	},
	#[deku(id = "0x2b")]
	Int32 {
		val: i32
	},
	#[deku(id = "0x2c")]
	Int40 {
		#[deku(bits = 40)]
		val: i64
	},
	#[deku(id = "0x2d")]
	Int48 {
		#[deku(bits = 48)]
		val: i64
	},
	#[deku(id = "0x2e")]
	Int56 {
		#[deku(bits = 56)]
		val: i64
	},
	#[deku(id = "0x2f")]
	Int64 {
		val: i64
	},
	#[deku(id = "0x38")]
	Semi {
		#[deku(bits = 16)]
		val: f32
	},
	#[deku(id = "0x39")]
	Single {
		val: f32
	},
	#[deku(id = "0x3a")]
	Double {
		val: f64
	},
	#[deku(id = "0xe0")]
	ToD {
		h: u8,
		m: u8,
		s: u8,
		cs: u8
	},
	#[deku(id = "0xe1")]
	Date {
		y: u8,
		m: u8,
		d: u8,
		dow: u8
	},
	#[deku(id = "0xe2")]
	UTC {
		val: u32
	},
}

#[derive(Debug, Serialize, Deserialize, DekuRead, DekuWrite, Clone)]
pub struct ReadAttributeRecord {
	pub identifier: u16,
	status: u8, //todo status enum
	#[deku(cond = "*status == 0")]
	pub data: Option<AttributeValue>
}

#[derive(Debug, Serialize, Deserialize, DekuRead, DekuWrite, Clone)]
pub struct AttributeRecord {
	pub identifier: u16,
	pub data: AttributeValue
}

#[derive(Debug, Serialize, Deserialize, DekuRead, DekuWrite, Clone)]
pub struct WriteAttributeStatus {
	status: u8,
	identifier: u16
}

#[derive(Debug, Serialize, Deserialize, DekuRead, DekuWrite, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
#[deku(id_type = "u8")]
pub enum ConfigureReportingDirection {
	#[deku(id = "0x00")]
	Report {
		identifier: u16,
		data_type: u8,
		min_interval: u16,
		max_interval: u16,
		#[deku(ctx = "*data_type")]
		change: AttributeChangeValue
	},
	#[deku(id = "0x01")]
	Receive {
		timeout: u16
	}
}

#[derive(Debug, Serialize, Deserialize, DekuRead, DekuWrite, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
#[deku(id_type = "u8")]
pub enum GenericCommand {
	#[deku(id = "0x00")]
	ReadAttributes {
		#[deku(read_all)]
		identifiers: Vec<u16>
	},
	#[deku(id = "0x01")]
	ReadAttributesResponse {
		#[deku(read_all)]
		values: Vec<ReadAttributeRecord>
	},
	#[deku(id = "0x02")]
	WriteAttributes {
		#[deku(read_all)]
		values: Vec<AttributeRecord>
	},
	#[deku(id = "0x03")]
	WriteAttributesUndivided {
		#[deku(read_all)]
		values: Vec<AttributeRecord>
	},
	#[deku(id = "0x04")]
	WriteAttributesResponse {
		#[deku(read_all)]
		values: Vec<WriteAttributeStatus>
	},
	#[deku(id = "0x05")]
	WriteAttributesNoResponse {
		#[deku(read_all)]
		values: Vec<AttributeRecord>
	},
	#[deku(id = "0x0a")]
	ReportAttributes {
		#[deku(read_all)]
		values: Vec<AttributeRecord>
	},
	#[deku(id = "0x06")]
	ConfigureReporting {
		direction: ConfigureReportingDirection,
	},

	//todo implement when needed
	#[deku(id = "0x07")]
	ConfigureReportingResponse {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x08")]
	ReadReportingConfiguration {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x09")]
	ReadReportingConfigurationResponse {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x0b")]
	DefaultResponse {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x0c")]
	DiscoverAttributes {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x0d")]
	DiscoverAttributesResponse {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x0e")]
	ReadAttributesStructured {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x0f")]
	WriteAttributesStructured {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x10")]
	WriteAttributesStructuredresponse {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x11")]
	DiscoverCommandsReceived {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x12")]
	DiscoverCommandsReceivedResponse {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x13")]
	DiscoverCommandsGenerated {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x14")]
	DiscoverCommandsGeneratedResponse {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x15")]
	DiscoverAttributesExtended {
		#[deku(read_all)]
		values: Vec<u8>
	},
	#[deku(id = "0x16")]
	DiscoverAttributesExtendedResponse {
		#[deku(read_all)]
		values: Vec<u8>
	},
}

//cluster 6 commands
#[derive(Serialize, Deserialize, Debug, DekuRead, DekuWrite, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
#[deku(id_type = "u8", bytes = "1")]
pub enum OnOffCommand {
	#[deku(id = "0x00")]
	Off,
	#[deku(id = "0x01")]
	On,
	#[deku(id = "0x02")]
	Toggle,
	#[deku(id = "0x40")]
	OffWithEffect{
		effect_identifier: u8,
		effect_variant: u8
	},
	#[deku(id = "0x41")]
	OnWithRecallGlobalScene,
	#[deku(id = "0x42")]
	OnWithTimedOff{
		on_off_control: u8,
		on_time: u16,
		off_wait_time: u16
	},
}

//cluster 8 commands
#[derive(Serialize, Deserialize, Debug, DekuRead, DekuWrite, Clone)]
#[serde(tag = "type")]
#[deku(id_type = "u8", bytes = "1")]
#[serde(rename_all = "snake_case")]
pub enum LevelCommand {
	#[deku(id = "0x00")]
	MoveToLevel{
		level: u8,
		#[serde(default)]
		transistion_time: u16,
		#[serde(default)]
		option_mask: u8,
		#[serde(default)]
		options_override: u8
	},
	#[deku(id = "0x01")]
	Move{
		move_mode: u8,
		rate: u8,
		option_mask: u8,
		options_override: u8
	},
	#[deku(id = "0x02")]
	Step{
		step_mode: u8,
		step_size: u8,
		transistion_time: u16,
		#[deku(read_all)]
		options: Vec<u8>
		//option_mask: u8,
		//options_override: u8
	},
	#[deku(id = "0x03")]
	Stop{
		option_mask: u8,
		options_override: u8
	},
	#[deku(id = "0x04")]
	MoveToLevelWithOnOff{
		level: u8,
		transistion_time: u16,
		option_mask: u8,
		options_override: u8
	},
	#[deku(id = "0x05")]
	MoveWithOnOff{
		move_mode: u8,
		rate: u8,
		option_mask: u8,
		options_override: u8
	},
	#[deku(id = "0x06")]
	StepWithOnOff{
		step_mode: u8,
		step_size: u8,
		transistion_time: u16,
		#[deku(read_all)]
		options: Vec<u8>
		//option_mask: u8,
		//options_override: u8
	},
	#[deku(id = "0x07")]
	StopWithOnOff{
		option_mask: u8,
		options_override: u8
	},
	#[deku(id = "0x08")]
	MoveToClosestFrequency{
		frequency: u16
	},
}

//cluster 0x0500 commands
#[derive(Serialize, Deserialize, Debug, DekuRead, DekuWrite, Clone)]
#[serde(tag = "type")]
#[deku(id_type = "u8", bytes = "1")]
#[serde(rename_all = "snake_case")]
pub enum ZoneCommand {
	#[deku(id = "0x00")]
	StatusChange{
		status: u16,
		extended_status: u8,
		zone_id: u8,
		delay: u16
	},
	#[deku(id = "0x01")]
	EnrollRequest{
		zone_type: u16,
		manufacturer_code: u16
	},
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
pub struct ZclFrameControl {
	#[deku(bits = 1, pad_bits_before = "3")]
	pub disable_default_response: bool,
	#[deku(bits = 1)]
	pub direction: u8,
	#[deku(bits = 1)]
	pub manufacturer_specific: u8,
	#[deku(bits = 2)]
	pub frame_type: u8,
}

//Frame Type Description
//00 Command is global for all clusters, including manufacturer specific clusters
//01 Command is specific or local to a cluster

#[derive(Debug, DekuWrite, Clone)]
#[deku(id_type = "u16", bytes = "0")]
pub enum Command {
	#[deku(id = "0x01")]
	Generic(GenericCommand),
	#[deku(id = "0x02")]
	Tuya(crate::tuya::TuyaCommand),
	#[deku(id = "0x03")]
	Raw(Vec<u8>),
	#[deku(id = "0x0006")]
	OnOffCommand(OnOffCommand),
	#[deku(id = "0x0008")]
	LevelCommand(LevelCommand),
	#[deku(id = "0x0500")]
	ZoneCommand(ZoneCommand),
}

impl Command {
	pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
		Ok(DekuContainerWrite::to_bytes(self)?)
	}

	pub fn to_json(&self) -> Result<serde_json::Value, Error> {
		Ok(match self {
			Self::Generic(g) => serde_json::to_value(g)?,
			Self::OnOffCommand(c) => serde_json::to_value(c)?,
			Self::LevelCommand(c) => serde_json::to_value(c)?,
			Self::ZoneCommand(c) => serde_json::to_value(c)?,
			_ => serde_json::Value::Null
		})
	}

	pub fn from_json(value: serde_json::Value, cluster_id: u16, frame_type: u8) -> Result<Self, Error> {
		match frame_type {
			1 => {
				match cluster_id {
					0x0006 => Ok(Self::OnOffCommand(serde_json::from_value::<OnOffCommand>(value)?)),
					0x0008 => Ok(Self::LevelCommand(serde_json::from_value::<LevelCommand>(value)?)),
					0x0500 => Ok(Self::ZoneCommand(serde_json::from_value::<ZoneCommand>(value)?)),
					_ => Err(Error::NotImplemented)
				}
			},
			0 => Ok(Self::Generic(serde_json::from_value::<GenericCommand>(value)?)),
			_ => Err(Error::NotImplemented)
		}
	}

	pub fn from_buf(buf: &[u8], frame_type: u8, cluster_id: u16, extension_type: CommandExtensionType) -> Result<Self, Error> {
		Ok(match frame_type {
			1 => {
				match extension_type {
					CommandExtensionType::Raw => {
						match cluster_id {
							0x0006 => Command::OnOffCommand(OnOffCommand::try_from(buf)?),
							0x0008 => Command::LevelCommand(LevelCommand::try_from(buf)?),
							0x0500 => Command::ZoneCommand(ZoneCommand::try_from(buf)?),
							_ => Command::Raw(buf.to_vec())
						}
					},
					CommandExtensionType::Tuya => {
						match cluster_id {
							0xef00 => Command::Tuya(crate::tuya::TuyaCommand::try_from(buf)?),
							_ => Command::Raw(buf.to_vec())
						}
					}
				}
			},
			_ => Command::Generic(GenericCommand::try_from(buf)?)
		})
	}
}

#[derive(Debug, DekuWrite, Clone)]
pub struct ZclFrame {
	pub control: ZclFrameControl,
	pub manufacturer_code: Option<u16>,
	pub transaction_sequence_number: u8,
	pub command: Command,
}

pub enum CommandExtensionType {
	Raw,
	Tuya
}

impl ZclFrame {
	pub fn from_command(command: Command) -> Self {
		Self {
			control: ZclFrameControl {
				disable_default_response: false,
				direction: 0,
				manufacturer_specific: 0,
				frame_type: 0
			},
			manufacturer_code: None,
			transaction_sequence_number: 0,
			command
		}
	}

	pub fn from_buf(buf: &[u8], cluster_id: u16, extension_type: CommandExtensionType) -> Result<Self, Error> {
		if buf.len() < 1 {
			return Err(Error::BufferTooSmall("Empty buffer"));
		}

		//read frameControl
		let control = ZclFrameControl::try_from(&buf[0..1])?;

		let mut pos = 1;
		let manufacturer_code = match control.manufacturer_specific {
			1 => {
				if buf.len() < 3 {
					return Err(Error::BufferTooSmall("Buffer length < 3"));
				}

				let c = buf[1] as u16 + (buf[2] as u16) << 8;
				pos += 2;

				Some(c)
			},
			_ => None
		};

		if buf.len() <= pos {
			return Err(Error::BufferTooSmall("No seq number"));
		}

		let transaction_sequence_number = buf[pos];

		pos += 1;

		if buf.len() <= pos {
			return Err(Error::BufferTooSmall("No command"));
		}

		let command = Command::from_buf(&buf[pos..], control.frame_type, cluster_id, extension_type)?;

		Ok(ZclFrame {
			control,
			manufacturer_code,
			transaction_sequence_number,
			command
		})
	}

	pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
		Ok(DekuContainerWrite::to_bytes(self)?)
	}
}






