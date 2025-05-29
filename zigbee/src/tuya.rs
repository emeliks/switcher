use deku::{ self, prelude::* };
use crate::zcl::AttributeValue;

#[derive(Debug, DekuRead, DekuWrite, Clone)]
pub struct DpData {
	pub dp_id: u8,
	pub value: DpDataValue
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum DpDataValue {
	#[deku(id = "0x00")]
	Raw {
		#[deku(endian = "big")]
		count: u16,
		#[deku(count = "count")]
		value: Vec<u8>
	},
	#[deku(id = "0x01")]
	Boolean {
		#[deku(endian = "big")]
		count: u16,
		value: bool
	},
	#[deku(id = "0x02")]
	Value {
		#[deku(endian = "big")]
		count: u16,
		#[deku(endian = "big")]
		value: u32
	},
	#[deku(id = "0x03")]
	String {
		#[deku(endian = "big")]
		count: u16,
		#[deku(count = "count")]
		value: Vec<u8>
	},
	#[deku(id = "0x04")]
	Enum {
		#[deku(endian = "big")]
		count: u16,
		value: u8
	},
	#[deku(id = "0x05")]
	Bitmap {
		#[deku(endian = "big")]
		count: u16,
		#[deku(count = "count")]
		value: Vec<u8>
	},
}

impl core::convert::Into<AttributeValue> for DpDataValue {
	fn into(self) -> AttributeValue {
		match self {
			Self::Raw { count, value } => AttributeValue::Octstr16 { count, val: value },
			Self::Boolean { value, .. } => AttributeValue::Bool { val: value },
			Self::Value { value, .. } => AttributeValue::Data32 { val: value },
			Self::String { count, value } => AttributeValue::String16 { count, val: value },
			Self::Enum { value, .. } => AttributeValue::Enum8 { val: value },
			Self::Bitmap { count, value } => AttributeValue::Octstr16 { count, val: value }
		}
	}
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum TuyaCommand {
	#[deku(id = "0x00")]
	TyDataRequest {
		//The gateway sends a data request to the Zigbee device.
		sequence_number: u16,
		data: DpData
	},
	#[deku(id = "0x01")]
	TyDataResponse {
		//The MCU responds to a data request.
		sequence_number: u16,
		data: DpData
	},
	#[deku(id = "0x02")]
	TyDataReport {
		//The MCU proactively reports data to the gateway. This is two-way communication.
		sequence_number: u16,
		data: DpData
	},
	#[deku(id = "0x03")]
	TyDataQuery, //The gateway sends a query to the Zigbee device for all the current information. The query does not contain a ZCL payload. We recommend that you set a reporting policy on the Zigbee device to avoid reporting all data in one task.
	#[deku(id = "0x10")]
	TuyaMcuVersionReq {
		//The gateway sends a query to the Zigbee module for the MCU firmware version.
		sequence_number: u16,
	},
	#[deku(id = "0x11")]
	TuyaMcuVersionRsp {
		//The Zigbee module returns the MCU firmware version, or the MCU proactively reports its firmware version.
		sequence_number: u16,
		version: u8
	},
	#[deku(id = "0x12")]
	TuyaMcuOtaNotify, //The gateway notifies the Zigbee module of MCU firmware updates.
	#[deku(id = "0x13")]
	TuyaOtaBlockDataReq, //The Zigbee module requests to download the MCU firmware update.
	#[deku(id = "0x14")]
	TuyaOtaBlockDataRsp {
		//The gateway returns the update package to the Zigbee module.
		sequence_number: u16,
		status: u8,
		key: [u8; 8],
		version: u8,
		offset: u32,
		#[deku(read_all)]
		image_data: Vec<u8>
	},
	#[deku(id = "0x15")]
	TuyaMcuOtaResult {
		//The Zigbee module returns the result of MCU firmware updates to the gateway.
		sequence_number: u16,
		status: u8,
		key: [u8; 8],
		version: u8,
	},
	#[deku(id = "0x24")]
	TuyaMcuSyncTime {
		//Time synchronization (two-way)
		sequence_number: u16,
		standard_timestamp: u32,
		local_timestamp: u32
	}
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
pub struct ZclTuyaFrame {
	pub control: crate::zcl::ZclFrameControl,
	pub transaction_sequence_number: u8,
	pub command: TuyaCommand,
}
