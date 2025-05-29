use deku::{ self, prelude::* };
use std::{ convert::{ TryFrom } };
use std::{ collections::VecDeque };
use frames::{ Frame, FrameRead, FrameWrite, FrameBuffer };

#[derive(Debug)]
pub enum Error {
	BufferTooShort,
	Deku(deku::DekuError),
	AshFormatError(&'static str),
	InitializationError,
	Frames(frames::Error),
	AshError{code: u8, version: u8}
}

impl core::fmt::Display for Error {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		match self {
			Self::BufferTooShort | Self::InitializationError => write!(f, "{self:?}"),
			Self::Deku(e) => e.fmt(f),
			Self::Frames(e) => e.fmt(f),
			Self::AshFormatError(s) => write!(f, "{}", s),
			Self::AshError{code, version} => write!(f, "Ash error (code: {:#02x}, version: {:#02x}), need reset", code, version)
		}
	}
}

impl From<deku::DekuError> for Error {
	fn from(e: deku::DekuError) -> Self {
		Error::Deku(e)
	}
}

impl From<frames::Error> for Error {
	fn from(e: frames::Error) -> Self {
		Error::Frames(e)
	}
}

pub struct AshProcessor<FS: FrameRead + FrameWrite> {
	stream: FS,
	state: AshProcessorState,
	legacy: bool,
	snd_frm_num: u8,	//to transmit
	rcv_ack_num: u8,	//received
	snd_ack_num: u8,	//to send
	pub sequence: u8,	//ezsp sequence
	buf_out: VecDeque<AshFrame>,
	nc_buf: Vec<CommandData>,	//temporary buffer for commands received in non connected state
	frame_in: FrameBuffer,
	frame_out: FrameBuffer,
	sequential_unicast: bool, //in case of errors after sending many unicast, turn it on
	unicast_counter: u8, //number of unresponded unicasts
}

#[derive(PartialEq)]
pub enum AshProcessorState {
	Init,
	RstSend,
	RstAck,
	VersionReqSend,
	Connected
}

impl<FS: FrameRead + FrameWrite + Sync + Send> AshProcessor<FS> {
	pub fn new(stream: FS) -> Self {
		Self {
			stream,
			state: AshProcessorState::Init,
			legacy: true,
			snd_frm_num: 0,
			rcv_ack_num: 0,
			snd_ack_num: 0,
			sequence: 0,
			buf_out: VecDeque::new(),
			nc_buf: Vec::new(),
			frame_in: FrameBuffer::default(),
			frame_out: FrameBuffer::default(),
			sequential_unicast: true,
			unicast_counter: 0
		}
	}

	fn create_data_frame(&mut self, data: CommandData) -> Result<AshFrame, Error> {
		let mut frame_format_version = if self.legacy { 0 } else { 1 };

		if let CommandData::Version{ .. } = &data {
			frame_format_version = 0;
		}

		let f = AshFrame {
			control: AshControl::Data(AshControlData {
				frm_num: 0, //will be set just before sending
				re_tx: 0,
				ack_num: 0, //will be set just before sending
			}),
			data: AshData::Ezsp(EzspFrame {
				sequence: 0, //will be set just before sending,
				control: FrameControl::Command(CommandControl {
					network_index: 0,
					sleep_mode: 0,
					security_enabled: 0,
					padding_enabled: 0,
					frame_format_version,
				}),
				data: FrameData::Command(data)
			})
		};

		Ok(f)
	}

	pub fn send_data_frame(&mut self, data: CommandData) -> Result<(), Error> {
		let f = self.create_data_frame(data)?;

		self.buf_out.push_back(f);

		Ok(())
	}

	pub fn run(&mut self, command_data: Option<CommandData>) -> Result<Option<(ResponseData, u8)>, Error> {
		let mut out = None;

		if let Some(command) = command_data {

			if let AshProcessorState::Connected = self.state {
				self.send_data_frame(command)?;
			}
			else {
				self.nc_buf.push(command);
			}
		}

		//need additional ack frame
		let mut need_ack = false;

		match AshFrame::nonblocking_read_frame(&mut self.stream, &mut self.frame_in, &self.legacy) {
			Ok(Some(frame)) => {
				//whole frame read
				//if data - push
				//if ack, nack - do transfer logic - todo

				match self.state {
					AshProcessorState::RstSend => {
						if let AshControl::Rstack = &frame.control {
							self.state = AshProcessorState::RstAck;
						}
					},
					AshProcessorState::VersionReqSend => {
						if let AshControl::Data(_) = &frame.control {
							self.state = AshProcessorState::Connected;
							self.legacy = false;

							//send remaining commands to buf
							for command in std::mem::take(&mut self.nc_buf) {
								self.send_data_frame(command)?;
							}
						}
					},
					_ => {}
				};

				match frame.control {
					AshControl::Data(racd) => {
						self.rcv_ack_num = racd.ack_num;

						//we have to ack received frame
						self.snd_ack_num = (racd.frm_num + 1) % 8;

						need_ack = true;

						//unicast counter
						if let AshData::Ezsp(EzspFrame { data: FrameData::Response(ResponseData::SendUnicast{ .. }), ..}) = frame.data {
							self.unicast_counter -= 1;
						}

						if let AshData::Ezsp(EzspFrame { sequence, data: FrameData::Response(response_data), ..}) = frame.data {
							out = Some((response_data, sequence));
						}
						else {
							println!("Unknown frame: {:?}", frame.data);
						}
					},
					AshControl::Ack(aca) => {
						self.rcv_ack_num = aca.ack_num;
					},
					AshControl::Nak(_aca) => {},
					AshControl::Rst => {},
					AshControl::Rstack => {},
					AshControl::Error => {
						let (code, version) = if let AshData::Error{code, version} = frame.data {(code, version)} else {(0, 0)};

						return Err(Error::AshError{code, version});
					}
				}
			},
			Ok(None) => {
			},
			Err(e) => {
				println!("Error read frame: {:?}", e);

				if e.need_reset() {
					return Err(Error::Frames(e));
				}
			}
		}

		if self.frame_out.is_empty() {

			let mut frame = match self.state {
				AshProcessorState::Init => {
					self.state = AshProcessorState::RstSend;

					Some(AshFrame {
						control: AshControl::Rst,
						data: AshData::None
					})
				},
				AshProcessorState::RstSend => { None },
				AshProcessorState::RstAck => {
					self.state = AshProcessorState::VersionReqSend;

					Some(self.create_data_frame(CommandData::Version {
						desired_protocol_version: 8
					})?)
				},
				AshProcessorState::VersionReqSend => None,
				AshProcessorState::Connected => {
					//only if sent no more than x frames unacknoweledged

					if (self.snd_frm_num - self.rcv_ack_num) % 8 <= 3 {
						if self.sequential_unicast && self.unicast_counter >= 1 {
							//skip unicast frames if unicast counter > max
							let mut f = None;

							for i in 0..self.buf_out.len() {
								if let Some(AshFrame{data: AshData::Ezsp(EzspFrame { data: FrameData::Command(CommandData::SendUnicast{ .. }), ..}), ..}) = self.buf_out.get(i) {
								}
								else {
									f = self.buf_out.remove(i);
								}
							}
							f
						}
						else {
							self.buf_out.pop_front()
						}
					}
					else {
						None
					}
				}
			};

			match frame {
				None => {
					if need_ack {
						frame = Some(AshFrame {
							control: AshControl::Ack(AshControlAck { ack_num: self.snd_ack_num, n_rdy: 0, res: 0 }),
							data: AshData::None
						});
					}
				},
				_ => {}
			}

			//not necessary
			//need_ack = false;

			if let Some(mut frame) = frame {
				if let AshFrame { control: AshControl::Data(ref mut sacd), ref mut data } = & mut frame {
					sacd.ack_num = self.snd_ack_num;
					sacd.frm_num = self.snd_frm_num;

					self.snd_frm_num = (self.snd_frm_num + 1) % 8;

					if let AshData::Ezsp(EzspFrame {ref mut sequence, ..}) = data {
						*sequence = self.sequence;

						self.sequence += 1;
					}
				};

				//unicast counter
				if let AshData::Ezsp(EzspFrame { data: FrameData::Command(CommandData::SendUnicast{ .. }), ..}) = frame.data {
					self.unicast_counter += 1;
				}

				self.frame_out.push_frame(&frame, &self.legacy)?;
			}
		}

		if !self.frame_out.is_empty() {
			if AshFrame::nonblocking_write_frame(&mut self.stream, &mut self.frame_out)? {
				//whole frame written
			}
		}

		//todo timeouts, retransmission, etc.
		Ok(out)
	}
}

#[derive(Debug, DekuRead, DekuWrite)]
pub struct CommandControl {
	#[deku(pad_bits_before = "1", bits = 2)]
	pub network_index: u8,
	#[deku(pad_bits_before = "3", bits = 2)]
	pub sleep_mode: u8,
	#[deku(bits = 1)]
	pub security_enabled: u8,
	#[deku(bits = 1)]
	pub padding_enabled: u8,
	#[deku(pad_bits_before = "4", bits = 2)]
	pub frame_format_version: u8,
}

#[derive(Debug, DekuRead, DekuWrite)]
pub struct ResponseControl {
	#[deku(pad_bits_before = "1", bits = 2)]
	pub network_index: u8,
	#[deku(bits = 2)]
	pub callback_type: u8,
	#[deku(bits = 1)]
	pub callback_pending: u8,
	#[deku(bits = 1)]
	pub truncated: u8,
	#[deku(bits = 1)]
	pub overflow: u8,
	#[deku(bits = 1)]
	pub security_enabled: u8,
	#[deku(bits = 1)]
	pub padding_enabled: u8,
	#[deku(pad_bits_before = "4", bits = 2)]
	pub frame_format_version: u8,
}

#[derive(Debug)]
pub enum FrameControl {
	Command(CommandControl),
	Response(ResponseControl)
}

#[derive(Debug)]
pub struct EzspFrame {
	pub sequence: u8,
	pub control: FrameControl,
	pub data: FrameData
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u16")]
pub enum CommandData {
	#[deku(id = "0")]
	Version {
		desired_protocol_version: u8	//The EZSP version the Host wishes to use. To successfully set the version and allow other commands, this must be same as EZSP_PROTOCOL_VERSION.
	},
	#[deku(id = "0x0002")]
	AddEndpoint {
		endpoint: u8,
		profile_id: u16,
		device_id: u16,
		app_flags: u8,
		input_cluster_count: u8,
		output_cluster_count: u8,
		#[deku(count = "input_cluster_count")]
		input_cluster_list: Vec<u16>,
		#[deku(count = "output_cluster_count")]
		output_cluster_list: Vec<u16>,
	},
	#[deku(id = "0x0005")]
	Nop,
	#[deku(id = "0x0010")]
	SetConcentrator {
		on: Bool,
		concentrator_type: u16,
		min_time: u16,
		max_time: u16,
		route_error_threshold: u8,
		delivery_failure_threshold: u8,
		max_hops: u8
	},
	#[deku(id = "0x0017")]
	NetworkInit(EmberNetworkInitStruct),
	#[deku(id = "0x001E")]
	FormNetwork(EmberNetworkParameters),
	#[deku(id = "0x0022")]
	PermitJoining {
		duration: u8
	},
	#[deku(id = "0x0026")]
	GetEui64,
	#[deku(id = "0x0028")]
	GetNetworkParameters,
	#[deku(id = "0x0034")]
	SendUnicast {
		r#type: EmberOutgoingMessageType,
		index_or_destination: EmberNodeId,
		aps_frame: EmberApsFrame,
		message_tag: u8,
		message_length: u8,
		#[deku(count = "message_length")]
		message_contents: Vec<u8>
	},
	#[deku(id = "0x0049")]
	GetRandomNumber,
	#[deku(id = "0x0053")]
	SetConfigurationValue {
		config_id: EzspConfigId,
		value: u16
	},
	#[deku(id = "0x0055")]
	SetPolicy {
		policy_id: EzspPolicyId,
		decision_id: EzspDecisionId
	},
	#[deku(id = "0x005F")]
	GetAddressTableRemoteNodeId{
		address_table_index: u8
	},
	#[deku(id = "0x0064")]
	SetMulticastTableEntry {
		index: u8,
		value: EmberMulticastTableEntry
	},
	#[deku(id = "0x0068")]
	SetInitialSecurityState {
		state: EmberInitialSecurityState
	},
	#[deku(id = "0x0069")]
	GetCurrentSecurityState,
	#[deku(id = "0x0081")]
	Echo {
		count: u8,
		#[deku(count = "count")]
		data: Vec<u8>,
	},
	#[deku(id = "0x00AB")]
	SetValue {
		value_id: EzspValueId,
		value_length: u8,
		#[deku(count = "value_length")]
		value: Vec<u8>
	}
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u16")]
pub enum ResponseData {
	#[deku(id = "0")]
	Version {
		protocol_version:u8,
		stack_type: u8,
		stack_version: u16
	},
	#[deku(id = "0x0002")]
	AddEndpoint {
		status: EzspStatus
	},
	#[deku(id = "0x0005")]
	Nop,
	#[deku(id = "0x000D")]
	StackTokenChangedHandler {
		token_address: u16
	},
	#[deku(id = "0x0010")]
	SetConcentrator {
		status: EmberStatus
	},
	#[deku(id = "0x0017")]
	NetworkInit {
		status: EmberStatus
	},
	#[deku(id = "0x001E")]
	FormNetwork {
		status: EmberStatus
	},
	#[deku(id = "0x0019")]
	StackStatusHandler {
		status: EmberStatus
	},
	#[deku(id = "0x0022")]
	PermitJoining {
		status: EmberStatus
	},
	#[deku(id = "0x0023")]
	ChildJoinHandler {
		index: u8,
		joining: Bool,
		child_id: EmberNodeId,
		child_eui64: EmberEUI64,
		child_type: EmberNodeType
	},
	#[deku(id = "0x0024")]
	TrustCenterJoinHandler {
		new_node_id: EmberNodeId,
		new_node_eui64: EmberEUI64,
		statue: EmberDeviceUpdate,
		policy_decision: EmberJoinDecision,
		parent_of_new_node_id: EmberNodeId
	},
	#[deku(id = "0x0026")]
	GetEui64 {
		eui64: u64
	},
	#[deku(id = "0x0028")]
	GetNetworkParameters {
		status: EmberStatus,
		node_type: EmberNodeType,
		parameters: EmberNetworkParameters
	},
	#[deku(id = "0x0034")]
	SendUnicast {
		status: EmberStatus,
		sequence: u8
	},
	#[deku(id = "0x003F")]
	MessageSentHandler {
		r#type: EmberOutgoingMessageType,
		index_or_destination: u16,
		aps_frame: EmberApsFrame,
		message_tag: u8,
		status: EmberStatus,
		message_length: u8,
		#[deku(count = "message_length")]
		message_contents: Vec<u8>
	},
	#[deku(id = "0x0044")]
	PollHandler {
		child_id: EmberNodeId,
		//only for version > 8
		//transmit_expected: Bool
		#[deku(read_all)]
		d: Vec<u8>
	},
	#[deku(id = "0x0045")]
	IncomingMessageHandler {
		r#type: EmberIncomingMessageType,
		aps_frame: EmberApsFrame,
		last_hop_lqi: u8,
		last_hop_rssi: i8,
		sender: EmberNodeId,
		binding_index: u8,
		address_index: u8,
		message_length: u8,
		#[deku(count = "message_length")]
		message_contents: Vec<u8>,
		//sometimes additional byte - probably for newer version
		#[deku(read_all)]
		d: Vec<u8>
	},
	#[deku(id = "0x0049")]
	GetRandomNumber {
		status: EmberStatus,
		value: u16
	},
	#[deku(id = "0x0053")]
	SetConfigurationValue {
		status: EzspStatus
	},
	#[deku(id = "0x0055")]
	SetPolicy {
		status: EzspStatus
	},
	#[deku(id = "0x0059")]
	IncomingRouteRecordHandler {
		#[deku(read_all)]
		d: Vec<u8>
	},
	#[deku(id = "0x005F")]
	GetAddressTableRemoteNodeId{
		node_id: EmberNodeId
	},
	#[deku(id = "0x0062")]
	IncomingSenderEui64Handler {
		sender_eui64: EmberEUI64
	},
	#[deku(id = "0x0064")]
	SetMulticastTableEntry {
		status: EmberStatus
	},
	#[deku(id = "0x0068")]
	SetInitialSecurityState {
		status: EmberStatus
	},
	#[deku(id = "0x0069")]
	GetCurrentSecurityState {
		status: EmberStatus,
		state: EmberCurrentSecurityState
	},
	#[deku(id = "0x0081")]
	IncomingRouteErrorHandler {
		status: EmberStatus,
		target: EmberNodeId
	},
	#[deku(id = "0x0081")]
	Echo{
		count: u8,
		#[deku(count = "count")]
		data: Vec<u8>,
	},
	#[deku(id = "0x009B")]
	ZigbeeKeyEstablishmentHandler {
			partner: EmberEUI64,
			status: EmberKeyStatus
	},
	#[deku(id = "0x00AB")]
	SetValue {
		status: EzspStatus
	},
}

//from tasmota
pub const Z_PROF_HA: u16 = 0x0104;
pub const EMBER_JOINER_GLOBAL_LINK_KEY: u16 = 0x0010;
pub const EMBER_NWK_LEAVE_REQUEST_NOT_ALLOWED: u16 = 0x0100;

#[derive(Debug)]
pub enum FrameData {
	Command(CommandData),
	Response(ResponseData)
}

impl EzspFrame {
	pub fn from_buf(buf: &[u8], legacy: bool) -> Result<Self, Error> {
		let mut ctrl_buf = [0; 2];

		if legacy {
			ctrl_buf[0] = buf[1];
		}
		else {
			ctrl_buf[0] = buf[1];
			ctrl_buf[1] = buf[2];
		}

		let is_command = buf[1] & 0b10000000 == 0;
		let mut data_buf = Vec::new();

		if legacy {
			data_buf.push(buf[2]);
			data_buf.push(0);
			data_buf.extend_from_slice(&buf[3..]);
		}
		else {
			data_buf.extend_from_slice(&buf[3..]);
		}

		Ok(EzspFrame {
			sequence: buf[0],
			control: if is_command { FrameControl::Command(CommandControl::try_from(ctrl_buf.as_slice())?) } else { FrameControl::Response(ResponseControl::try_from(ctrl_buf.as_slice())?)},
			data: if is_command { FrameData::Command(CommandData::try_from(data_buf.as_slice())?) } else { FrameData::Response(ResponseData::try_from(data_buf.as_slice())?) }
		})
	}

	pub fn as_bytes(&self, legacy: bool, buf: &mut Vec<u8>) -> Result<(), Error> {
		buf.push(self.sequence);

		let mut ctrl_buf = match &self.control {
			FrameControl::Command(cc) => {
				let mut buf = cc.to_bytes()?;
				buf[0] = buf[0] & !0b10000000;

				buf
			},
			FrameControl::Response(rc) => {
				let mut buf = rc.to_bytes()?;
				buf[0] = buf[0] | 0b10000000;

				buf
			}
		};

		if legacy {
			buf.push(ctrl_buf[0]);
		}
		else {
			buf.append(&mut ctrl_buf);
		}

		let mut data_buf = match &self.data {
			FrameData::Command(cd) => cd.to_bytes()?,
			FrameData::Response(rd) => rd.to_bytes()?
		};

		if legacy {
			buf.push(data_buf[0]);
			buf.extend_from_slice(&data_buf[2..]);
		}
		else {
			buf.append(&mut data_buf);
		}

		Ok(())
	}
}

#[derive(Debug, DekuRead, DekuWrite)]
pub struct AshControlAck {
	#[deku(pad_bits_before = "3", bits = 1)]
	pub res: u8,
	#[deku(bits = 1)]
	pub n_rdy: u8,
	#[deku(bits = 3)]
	pub ack_num: u8,
}

#[derive(Debug, DekuRead, DekuWrite)]
pub struct AshControlData {
	#[deku(pad_bits_before = "1", bits = 3)]
	pub frm_num: u8,
	#[deku(bits = 1)]
	pub re_tx: u8,
	#[deku(bits = 3)]
	pub ack_num: u8,
}

const CONTROL_RST: u8 = 0b11000000;
const CONTROL_RSTACK: u8 = 0b11000001;
const CONTROL_ERROR: u8 = 0b11000010;
const CONTROL_ACK_MASK: u8 = 0b11100000;
const CONTROL_ACK: u8 = 0b10000000;
const CONTROL_NAK: u8 = 0b10100000;
const CONTROL_DATA_MASK: u8 = 0b10000000;

#[derive(Debug)]
pub enum AshControl {
	Data(AshControlData),
	Ack(AshControlAck),
	Nak(AshControlAck),
	Rst,
	Rstack,
	Error
}

#[derive(Debug)]
pub enum AshData {
	Ezsp(EzspFrame),
	Rstack {
		version: u8,
		code: u8
	},
	Error {
		version: u8,
		code: u8
	},
	None,
	ParseError(Error, Vec<u8>)
}

#[derive(Debug)]
pub struct AshFrame {
	pub control: AshControl,
	pub data: AshData
}

pub struct Random {
	val: u8
}

impl Random {
	fn new() -> Self {
		Random {
			val: 0
		}
	}

	fn next(&mut self) -> u8 {
		if self.val == 0 {
			self.val = 0x42;
		}
		else {
			self.val = if self.val & 1 == 0 { self.val >> 1 } else { (self.val >> 1) ^ 0xb8 };
		}

		self.val
	}
}

impl From<Error> for frames::Error {
	fn from(r: Error) -> Self {
		frames::Error::Other(r.to_string())
	}
}

impl frames::Frame for AshFrame {
	type Params = bool;

	fn get_buffer_len(buf: &mut Vec<u8>, _params: &Self::Params) -> Result<usize, frames::Error> {
		Ok(AshFrame::get_buffer_len(buf)?)
	}

	fn from_buf(buf: &[u8], params: &Self::Params) -> Result<Self, frames::Error> {
		Ok(AshFrame::from_buf(buf, *params)?)
	}

	fn as_bytes(&self, params: &Self::Params, buf: &mut Vec<u8>) -> Result<(), frames::Error> {
		Ok(AshFrame::as_bytes(&self, *params, buf)?)
	}
}

impl AshFrame {
	pub fn get_buffer_len(buf: &mut Vec<u8>) -> Result<usize, Error> {
		//control bytes
		match buf.last() {
			Some(0x11) | Some(0x13) => { buf.pop(); },
			Some(0x1a) => { buf.clear(); },
			_ => {}
		};

		//todo 0x18 - substitute byte

		//wait until 0x7e at end of frame
		Ok(if let Some(0x7e) = buf.last() { 0 } else { 1 })
	}

	pub fn crc16(buf: &[u8]) -> u16 {
		let mut crc16: u16 = 0xffff;

		for i in 0..buf.len() {
			let out_byte = buf[i] as u16;

			crc16 = crc16 ^ (out_byte << 8);

			for _ in 0..8 {
				if (crc16 & 0x8000) != 0 {
					crc16 = (crc16 << 1) ^ 0x1021;
				} else {
					crc16 <<= 1;
				}
			}
		}

		crc16
	}

	pub fn from_buf(buf: &[u8], legacy: bool) -> Result<Self, Error> {
		//End Flag

		if let Some(0x7e) = buf.last() {
		}
		else {
			return Err(Error::AshFormatError("Bad frame ending"));
		}

		let mut buf_len = buf.len();

		if buf_len < 4 {
			return Err(Error::AshFormatError("Frame too small"));
		}

		//Byte de-stuffing
		let mut buf_d = Vec::new();

		let mut esc = false;

		for b in buf {
			if esc {
				buf_d.push(b ^ 0b00100000);
				esc = false;
			}
			else {
				if *b == 0x7d {
					esc = true;
				}
				else {
					buf_d.push(*b);
				}
			}
		}

		buf_len = buf_d.len();

		let crc16 = Self::crc16(&buf_d[0..buf_len - 3]);

		if crc16 != ((buf_d[buf_len - 3] as u16) << 8) + buf_d[buf_len - 2] as u16 {
			return Err(Error::AshFormatError("Bad CRC"));
		}

		//control byte
		let control_byte = buf_d[0];
		let control;
		let mut data = AshData::None;

		if control_byte & CONTROL_DATA_MASK == 0 {

			//Data derandomization
			let mut r = Random::new();

			for i in 1..buf_len - 3 {
				buf_d[i] = buf_d[i] ^ r.next();
			}

			//data
			let (_rest, acd) = AshControlData::from_bytes((&buf_d, 0))?;
			control = AshControl::Data(acd);

			let e_buf = &buf_d[1..buf_len - 3];

			if e_buf.len() == 0 {
				data = AshData::None;
			}
			else {
				data = match EzspFrame::from_buf(&e_buf, legacy) {
					Ok(ef) => AshData::Ezsp(ef),
					//needt to return parse error because have to ack ash frame
					Err(e) => AshData::ParseError(e, e_buf.to_vec())
				}
			}
		}
		else {
			//other
			if control_byte & CONTROL_ACK_MASK == CONTROL_ACK {
				//ack
				let (_rest, aca) = AshControlAck::from_bytes((&buf_d, 0))?;
				control = AshControl::Ack(aca);
			}
			else {
				if control_byte & CONTROL_ACK_MASK == CONTROL_NAK {
					//nak
					let (_rest, aca) = AshControlAck::from_bytes((&buf_d, 0))?;
					control = AshControl::Nak(aca);
				}
				else {
					match control_byte {
						CONTROL_RST => {
							//rst
							control = AshControl::Rst;
						},
						CONTROL_RSTACK => {
							//rstack
							control = AshControl::Rstack;
							data = AshData::Rstack {
								version: buf_d[1],
								code: buf_d[2]
							};
						},
						CONTROL_ERROR => {
							//error
							control = AshControl::Error;
							data = AshData::Error {
								version: buf_d[1],
								code: buf_d[2]
							};
						},
						_ => {
							return Err(Error::AshFormatError("Bad control byte"));
						}
					}
				}
			}
		}

		Ok(AshFrame {
			control,
			data
		})
	}

	pub fn as_bytes(&self, legacy: bool, buf: &mut Vec<u8>) -> Result<(), Error> {
		let mut buf_d = Vec::new();

		//Control byte
		let control_byte = match &self.control {
			AshControl::Data(acd) => acd.to_bytes()?[0] & !CONTROL_DATA_MASK,
			AshControl::Ack(aca) => aca.to_bytes()?[0] & !CONTROL_ACK_MASK | CONTROL_ACK,
			AshControl::Nak(aca) => aca.to_bytes()?[0] & !CONTROL_ACK_MASK | CONTROL_NAK,
			AshControl::Rst => CONTROL_RST,
			AshControl::Rstack => CONTROL_RSTACK,
			AshControl::Error => CONTROL_ERROR
		};

		buf_d.push(control_byte);

		//Data
		match &self.data {
			AshData::None => {},
			AshData::ParseError(..) => {},
			AshData::Ezsp(ef) => {
				ef.as_bytes(legacy, &mut buf_d)?;

				//Data randomization
				let mut r = Random::new();

				for i in 1..buf_d.len() {
					buf_d[i] = buf_d[i] ^ r.next();
				}
			},
			AshData::Rstack { version, code } => {
				buf.push(*version);
				buf.push(*code);
			},
			AshData::Error { version, code } => {
				buf.push(*version);
				buf.push(*code);
			}
		};

		//crc16
		let crc16 = Self::crc16(&buf_d);

		buf_d.push((crc16 >> 8) as u8);
		buf_d.push(crc16 as u8);

		//Byte stuffing
		for b in buf_d {
			match b {
				0x7e | 0x11 | 0x13 | 0x18 | 0x1a | 0x7d => {
					buf.push(0x7d);
					buf.push(b ^ 0b00100000);
				},
				_ => {
					buf.push(b);
				}
			}
		}

		//End Flag
		buf.push(0x7e);

		Ok(())
	}
}

//ezsp types

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum EmberStatus {
	#[deku(id = "0x00")]
	EmberSuccess, //The generic 'no error' message.
	#[deku(id = "0x01")]
	EmberErrFatal, //The generic 'fatal error' message.
	#[deku(id = "0x02")]
	EmberBadArgument, //An invalid value was passed as an argument to a function
	#[deku(id = "0x04")]
	EmberEepromMfgStackVersionMismatch, //The manufacturing and stack token format in non-volatile memory is different than what the stack expects (returned at initialization).
	#[deku(id = "0x06")]
	EmberEepromMfgVersionMismatch, //The manufacturing token format in non-volatile memory is different than what the stack expects (returned at initialization).
	#[deku(id = "0x07")]
	EmberEepromStackVersionMismatch, //The stack token format in non-volatile memory is different than what the stack expects (returned at initialization).
	#[deku(id = "0x18")]
	EmberNoBuffers, //There are no more buffers.
	#[deku(id = "0x20")]
	EmberSerialInvalidBaudRate, //Specified an invalid baud rate.
	#[deku(id = "0x21")]
	EmberSerialInvalidPort, //Specified an invalid serial port.
	#[deku(id = "0x22")]
	EmberSerialTxOverflow, //Tried to send too much data.
	#[deku(id = "0x23")]
	EmberSerialRxOverflow, //There was not enough space to store a received character and the character was dropped.
	#[deku(id = "0x24")]
	EmberSerialRxFrameError, //Detected a UART framing error.
	#[deku(id = "0x25")]
	EmberSerialRxParityError, //Detected a UART parity error.
	#[deku(id = "0x26")]
	EmberSerialRxEmpty, //There is no received data to process.
	#[deku(id = "0x27")]
	EmberSerialRxOverrunError, //The receive interrupt was not handled in time, and a character was dropped.
	#[deku(id = "0x39")]
	EmberMacTransmitQueueFull, //The MAC transmit queue is full.
	#[deku(id = "0x3A")]
	EmberMacUnknownHeaderType, //MAC header FCR error on receive.
	#[deku(id = "0x3D")]
	EmberMacScanning, //The MAC can't complete this task because it is scanning.
	#[deku(id = "0x31")]
	EmberMacNoData, //No pending data exists for device doing a data poll.
	#[deku(id = "0x32")]
	EmberMacJoinedNetwork, //Attempt to scan when we are joined to a network.
	#[deku(id = "0x33")]
	EmberMacBadScanDuration, //Scan duration must be 0 to 14 inclusive. Attempt was made to scan with an incorrect duration value.
	#[deku(id = "0x34")]
	EmberMacIncorrectScanType, //emberStartScan was called with an incorrect scan type.
	#[deku(id = "0x35")]
	EmberMacInvalidChannelMask, //emberStartScan was called with an invalid channel mask.
	#[deku(id = "0x36")]
	EmberMacCommandTransmitFailure, //Failed to scan current channel because we were unable to transmit the relevant MAC command.
	#[deku(id = "0x40")]
	EmberMacNoAckReceived, //We expected to receive an ACK following the transmission, but the MAC level ACK was never received.
	#[deku(id = "0x42")]
	EmberMacIndirectTimeout, //Indirect data message timed out before polled.
	#[deku(id = "0x43")]
	EmberSimEepromErasePageGreen, //The Simulated EEPROM is telling the application that there is at least one flash page to be erased. The GREEN status means the current page has not filled above the ERASE_CRITICAL_THRESHOLD. The application should call the function halSimEepromErasePage when it can to erase a page.
	#[deku(id = "0x44")]
	EmberSimEepromErasePageRed, //The Simulated EEPROM is telling the application that there is at least one flash page to be erased. The RED status means the current page has filled above the ERASE_CRITICAL_THRESHOLD. Due to the shrinking availability of write space, there is a danger of data loss. The application must call the function halSimEepromErasePage as soon as possible to erase a page.
	#[deku(id = "0x45")]
	EmberSimEepromFull, //The Simulated EEPROM has run out of room to write any new data and the data trying to be set has been lost. This error code is the result of ignoring the SIM_EEPROM_ERASE_PAGE_RED error code. The application must call the function halSimEepromErasePage to make room for any further calls to set a token.
	#[deku(id = "0x46")]
	EmberErrFlashWriteInhibited, //A fatal error has occurred while trying to write data to the Flash. The target memory attempting to be programmed is already programmed. The flash write routines were asked to flip a bit from a 0 to 1, which is physically impossible and the write was therefore inhibited. The data in the flash cannot be trusted after this error.
	#[deku(id = "0x47")]
	EmberErrFlashVerifyFailed, //A fatal error has occurred while trying to write data to the Flash and the write verification has failed. The data in the flash cannot be trusted after this error, and it is possible this error is the result of exceeding the life cycles of the flash.
	#[deku(id = "0x48")]
	EmberSimEepromInit1Failed, //Attempt 1 to initialize the Simulated EEPROM has failed. This failure means the information already stored in Flash (or a lack thereof), is fatally incompatible with the token information compiled into the code image being run.
	#[deku(id = "0x49")]
	EmberSimEepromInit2Failed, //Attempt 2 to initialize the Simulated EEPROM has failed. This failure means Attempt 1 failed, and the token system failed to properly reload default tokens and reset the Simulated EEPROM.
	#[deku(id = "0x4A")]
	EmberSimEepromInit3Failed, //Attempt 3 to initialize the Simulated EEPROM has failed. This failure means one or both of the tokens TOKEN_MFG_NVDATA_VERSION or TOKEN_STACK_NVDATA_VERSION were incorrect and the token system failed to properly reload default tokens and reset the Simulated EEPROM.
	#[deku(id = "0x4B")]
	EmberErrFlashProgFail, //A fatal error has occurred while trying to write data to the flash, possibly due to write protection or an invalid address. The data in the flash cannot be trusted after this error, and it is possible this error is the result of exceeding the life cycles of the flash.
	#[deku(id = "0x4C")]
	EmberErrFlashEraseFail, //A fatal error has occurred while trying to erase flash, possibly due to write protection. The data in the flash cannot be trusted after this error, and it is possible this error is the result of exceeding the life cycles of the flash.
	#[deku(id = "0x58")]
	EmberErrBootloaderTrapTableBad, //The bootloader received an invalid message (failed attempt to go into bootloader).
	#[deku(id = "0x59")]
	EmberErrBootloaderTrapUnknown, //Bootloader received an invalid message (failed attempt to go into bootloader).
	#[deku(id = "0x5A")]
	EmberErrBootloaderNoImage, //The bootloader cannot complete the bootload operation because either an image was not found or the image exceeded memory bounds.
	#[deku(id = "0x66")]
	EmberDeliveryFailed, //The APS layer attempted to send or deliver a message, but it failed.
	#[deku(id = "0x69")]
	EmberBindingIndexOutOfRange, //This binding index is out of range of the current binding table.
	#[deku(id = "0x6A")]
	EmberAddressTableIndexOutOfRange, //This address table index is out of range for the current address table.
	#[deku(id = "0x6C")]
	EmberInvalidBindingIndex, //An invalid binding table index was given to a function.
	#[deku(id = "0x70")]
	EmberInvalidCall, //The API call is not allowed given the current state of the stack.
	#[deku(id = "0x71")]
	EmberCostNotKnown, //The link cost to a node is not known.
	#[deku(id = "0x72")]
	EmberMaxMessageLimitReached, //The maximum number of in-flight messages (i.e. EMBER_APS_UNICAST_MESSAGE_COUNT) has been reached.
	#[deku(id = "0x74")]
	EmberMessageTooLong, //The message to be transmitted is too big to fit into a single over-the-air packet.
	#[deku(id = "0x75")]
	EmberBindingIsActive, //The application is trying to delete or overwrite a binding that is in use.
	#[deku(id = "0x76")]
	EmberAddressTableEntryIsActive, //The application is trying to overwrite an address table entry that is in use.
	#[deku(id = "0x80")]
	EmberAdcConversionDone, //Conversion is complete.
	#[deku(id = "0x81")]
	EmberAdcConversionBusy, //Conversion cannot be done because a request is being processed.
	#[deku(id = "0x82")]
	EmberAdcConversionDeferred, //Conversion is deferred until the current request has been processed.
	#[deku(id = "0x84")]
	EmberAdcNoConversionPending, //No results are pending.
	#[deku(id = "0x85")]
	EmberSleepInterrupted, //Sleeping (for a duration) has been abnormally interrupted and exited prematurely.
	#[deku(id = "0x88")]
	EmberPhyTxUnderflow, //The transmit hardware buffer underflowed.
	#[deku(id = "0x89")]
	EmberPhyTxIncomplete, //The transmit hardware did not finish transmitting a packet.
	#[deku(id = "0x8A")]
	EmberPhyInvalidChannel, //An unsupported channel setting was specified.
	#[deku(id = "0x8B")]
	EmberPhyInvalidPower, //An unsupported power setting was specified.
	#[deku(id = "0x8C")]
	EmberPhyTxBusy, //The packet cannot be transmitted because the physical MAC layer is currently transmitting a packet. (This is used for the MAC backoff algorithm.)
	#[deku(id = "0x8D")]
	EmberPhyTxCcaFail, //The transmit attempt failed because all CCA attempts indicated that the channel was busy
	#[deku(id = "0x8E")]
	EmberPhyOscillatorCheckFailed, //The software installed on the hardware doesn't recognize the hardware radio type.
	#[deku(id = "0x8F")]
	EmberPhyAckReceived, //The expected ACK was received after the last transmission.
	#[deku(id = "0x90")]
	EmberNetworkUp, //The stack software has completed initialization and is ready to send and receive packets over the air.
	#[deku(id = "0x91")]
	EmberNetworkDown, //The network is not operating.
	#[deku(id = "0x94")]
	EmberJoinFailed, //An attempt to join a network failed.
	#[deku(id = "0x96")]
	EmberMoveFailed, //After moving, a mobile node's attempt to re-establish contact with the network failed.
	#[deku(id = "0x98")]
	EmberCannotJoinAsRouter, //An attempt to join as a router failed due to a ZigBee versus ZigBee Pro incompatibility. ZigBee devices joining ZigBee Pro networks (or vice versa) must join as End Devices, not Routers.
	#[deku(id = "0x99")]
	EmberNodeIdChanged, //The local node ID has changed. The application can obtain the new node ID by calling emberGetNodeId().
	#[deku(id = "0x9A")]
	EmberPanIdChanged, //The local PAN ID has changed. The application can obtain the new PAN ID by calling emberGetPanId().
	#[deku(id = "0x9C")]
	EmberNetworkOpened, //The network has been opened for joining.
	#[deku(id = "0x9D")]
	EmberNetworkClosed, //The network has been closed for joining.
	#[deku(id = "0xAB")]
	EmberNoBeacons, //An attempt to join or rejoin the network failed because no router beacons could be heard by the joining node.
	#[deku(id = "0xAC")]
	EmberReceivedKeyInTheClear, //An attempt was made to join a Secured Network using a pre-configured key, but the Trust Center sent back a Network Key in-the-clear when an encrypted Network Key was required.
	#[deku(id = "0xAD")]
	EmberNoNetworkKeyReceived, //An attempt was made to join a Secured Network, but the device did not receive a Network Key.
	#[deku(id = "0xAE")]
	EmberNoLinkKeyReceived, //After a device joined a Secured Network, a Link Key was requested but no response was ever received.
	#[deku(id = "0xAF")]
	EmberPreconfiguredKeyRequired, //An attempt was made to join a Secured Network without a pre-configured key, but the Trust Center sent encrypted data using a pre-configured key.
	#[deku(id = "0x93")]
	EmberNotJoined, //The node has not joined a network.
	#[deku(id = "0x95")]
	EmberInvalidSecurityLevel, //The chosen security level (the value of EMBER_SECURITY_LEVEL) is not supported by the stack.
	#[deku(id = "0xA1")]
	EmberNetworkBusy, //A message cannot be sent because the network is currently overloaded.
	#[deku(id = "0xA3")]
	EmberInvalidEndpoint, //The application tried to send a message using an endpoint that it has not defined.
	#[deku(id = "0xA4")]
	EmberBindingHasChanged, //The application tried to use a binding that has been remotely modified and the change has not yet been reported to the application.
	#[deku(id = "0xA5")]
	EmberInsufficientRandomData, //An attempt to generate random bytes failed because of insufficient random data from the radio.
	#[deku(id = "0xA6")]
	EmberApsEncryptionError, //There was an error in trying to encrypt at the APS Level. This could result from either an inability to determine the long address of the recipient from the short address (no entry in the binding table) or there is no link key entry in the table associated with the destination, or there was a failure to load the correct key into the encryption core.
	#[deku(id = "0xA8")]
	EmberSecurityStateNotSet, //There was an attempt to form or join a network with security without calling emberSetInitialSecurityState() first.
	#[deku(id = "0xB3")]
	EmberKeyTableInvalidAddress, //There was an attempt to set an entry in the key table using an invalid long address. An entry cannot be set using either the local device's or Trust Center's IEEE address. Or an entry already exists in the table with the same IEEE address. An Address of all zeros or all F's are not valid addresses in 802.15.4.
	#[deku(id = "0xB7")]
	EmberSecurityConfigurationInvalid, //There was an attempt to set a security configuration that is not valid given the other security settings.
	#[deku(id = "0xB8")]
	EmberTooSoonForSwitchKey, //There was an attempt to broadcast a key switch too quickly after broadcasting the next network key. The Trust Center must wait at least a period equal to the broadcast timeout so that all routers have a chance to receive the broadcast of the new network key. 
	#[deku(id = "0xBB")]
	EmberKeyNotAuthorized, //The message could not be sent because the link key corresponding to the destination is not authorized for use in APS data messages. APS Commands (sent by the stack) are allowed. To use it for encryption of APS data messages it must be authorized using a key agreement protocol (such as CBKE).
	#[deku(id = "0xBD")]
	EmberSecurityDataInvalid, //The security data provided was not valid, or an integrity check failed.
	#[deku(id = "0xA9")]
	EmberSourceRouteFailure, //A ZigBee route error command frame was received indicating that a source routed message from this node failed en route.
	#[deku(id = "0xAA")]
	EmberManyToOneRouteFailure, //A ZigBee route error command frame was received indicating that a message sent to this node along a many-to-one route failed en route. The route error frame was delivered by an ad-hoc search for a functioning route.
	#[deku(id = "0xB0")]
	EmberStackAndHardwareMismatch, //A critical and fatal error indicating that the version of the stack trying to run does not match with the chip it is running on. The software (stack) on the chip must be replaced with software that is compatible with the chip.
	#[deku(id = "0xB1")]
	EmberIndexOutOfRange, //An index was passed into the function that was larger than the valid range.
	#[deku(id = "0xB4")]
	EmberTableFull, //There are no empty entries left in the table.
	#[deku(id = "0xB6")]
	EmberTableEntryErased, //The requested table entry has been erased and contains no valid data.
	#[deku(id = "0xB5")]
	EmberLibraryNotPresent, //The requested function cannot be executed because the library that contains the necessary functionality is not present.
	#[deku(id = "0xBA")]
	EmberOperationInProgress, //The stack accepted the command and is currently processing the request. The results will be returned via an appropriate handler.
	#[deku(id = "0xF0")]
	EmberApplicationError0, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xF1")]
	EmberApplicationError1, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xF2")]
	EmberApplicationError2, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xF3")]
	EmberApplicationError3, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xF4")]
	EmberApplicationError4, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xF5")]
	EmberApplicationError5, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xF6")]
	EmberApplicationError6, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xF7")]
	EmberApplicationError7, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xF8")]
	EmberApplicationError8, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xF9")]
	EmberApplicationError9, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xFA")]
	EmberApplicationError10, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xFB")]
	EmberApplicationError11, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xFC")]
	EmberApplicationError12, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xFD")]
	EmberApplicationError13, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xFE")]
	EmberApplicationError14, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id = "0xFF")]
	EmberApplicationError15, //This error is reserved for customer application use. This will never be returned from any portion of the network stack or HAL.
	#[deku(id_pat = "_")]
	Unknown(u8),
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum EzspConfigId {
	#[deku(id = "0x01")]
	EzspConfigPacketBufferCount, //The NCP no longer supports configuration of packet buffer count at runtime using this parameter. Packet buffers must be configured using the EMBER_PACKET_BUFFER_COUNT macro when building the NCP project.
	#[deku(id = "0x02")]
	EzspConfigNeighborTableSize, //The maximum number of router neighbors the stack can keep track of. A neighbor is a node within radio range. 
	#[deku(id = "0x03")]
	EzspConfigApsUnicastMessageCount, //The maximum number of APS retried messages the stack can be transmitting at any time.
	#[deku(id = "0x04")]
	EzspConfigBindingTableSize, //The maximum number of non-volatile bindings supported by the stack.
	#[deku(id = "0x05")]
	EzspConfigAddressTableSize, //The maximum number of EUI64 to network address associations that the stack can maintain for the application. (Note, the total number of such address associations maintained by the NCP is the sum of the value of this setting and the value of ::EZSP_CONFIG_TRUST_CENTER_ADDRESS_CACH E_SIZE.).
	#[deku(id = "0x06")]
	EzspConfigMulticastTableSize, //The maximum number of multicast groups that the device may be a member of.
	#[deku(id = "0x07")]
	EzspConfigRouteTableSize, //The maximum number of destinations to which a node can route messages. This includes both messages originating at this node and those relayed for others.
	#[deku(id = "0x08")]
	EzspConfigDiscoveryTableSize, //The number of simultaneous route discoveries that a node will support.
	#[deku(id = "0x0C")]
	EzspConfigStackProfile, //Specifies the stack profile.
	#[deku(id = "0x0D")]
	EzspConfigSecurityLevel, //The security level used for security at the MAC and network layers. The supported values are 0 (no security) and 5 (payload is encrypted and a four-byte MIC is used for authentication).
	#[deku(id = "0x10")]
	EzspConfigMaxHops, //The maximum number of hops for a message.
	#[deku(id = "0x11")]
	EzspConfigMaxEndDeviceChildren, //The maximum number of end device children that a router will support.
	#[deku(id = "0x12")]
	EzspConfigIndirectTransmissionTimeout, //The maximum amount of time that the MAC will hold a message for indirect transmission to a child.
	#[deku(id = "0x13")]
	EzspConfigEndDevicePollTimeout, //The maximum amount of time that an end device child can wait between polls. If no poll is heard within this timeout, then the parent removes the end device from its tables. Value range 0-14. The timeout corresponding to a value of zero is 10 seconds. The timeout corresponding to a nonzero value N is 2^N minutes, ranging from 2^1 = 2 minutes to 2^14 = 16384 minutes.
	#[deku(id = "0x17")]
	EzspConfigTxPowerMode, //Enables boost power mode and/or the alternate transmitter output.
	#[deku(id = "0x18")]
	EzspConfigDisableRelay, //0: Allow this node to relay messages. 1: Prevent this node from relaying messages.
	#[deku(id = "0x19")]
	EzspConfigTrustCenterAddressCacheSize, //The maximum number of EUI64 to network address associations that the Trust Center can maintain. These address cache entries are reserved for and reused by the Trust Center when processing device join/rejoin authentications. This cache size limits the number of overlapping joins the Trust Center can process within a narrow time window (e.g. two seconds), and thus should be set to the maximum number of near simultaneous joins the Trust Center is expected to accommodate. (Note, the total number of such address associations maintained by the NCP is the sum of the value of this setting and the value of ::EZSP_CONFIG_ADDRESS_TABLE_SIZE.)
	#[deku(id = "0x1A")]
	EzspConfigSourceRouteTableSize, //The size of the source route table.
	#[deku(id = "0x1C")]
	EzspConfigFragmentWindowSize, //The number of blocks of a fragmented message that can be sent in a single window.
	#[deku(id = "0x1D")]
	EzspConfigFragmentDelayMs, //The time the stack will wait (in milliseconds) between sending blocks of a fragmented message.
	#[deku(id = "0x1E")]
	EzspConfigKeyTableSize, //The size of the Key Table used for storing individual link keys (if the device is a Trust Center) or Application Link Keys (if the device is a normal node).
	#[deku(id = "0x1F")]
	EzspConfigApsAckTimeout, //The APS ACK timeout value. The stack waits this amount of time between resends of APS retried messages.
	#[deku(id = "0x20")]
	EzspConfigBeaconJitterDuration, //The duration of a beacon jitter, in the units used by the 15.4 scan parameter (((1 << duration) + 1) * 15ms), when responding to a beacon request.
	#[deku(id = "0x22")]
	EzspConfigPanIdConflictReportThreshold, //The number of PAN id conflict reports that must be received by the network manager within one minute to trigger a PAN id change.
	#[deku(id = "0x24")]
	EzspConfigRequestKeyTimeout, //The timeout value in minutes for how long the Trust Center or a normal node waits for the ZigBee Request Key to complete. On the Trust Center this controls whether or not the device buffers the request, waiting for a matching pair of ZigBee Request Key. If the value is non-zero, the Trust Center buffers and waits for that amount of time. If the value is zero, the Trust Center does not buffer the request and immediately responds to the request. Zero is the most compliant behavior.
	#[deku(id = "0x29")]
	EzspConfigCertificateTableSize, //This value indicates the size of the runtime modifiable certificate table. Normally certificates are stored in MFG tokens but this table can be used to field upgrade devices with new Smart Energy certificates. This value cannot be set, it can only be queried.
	#[deku(id = "0x2A")]
	EzspConfigApplicationZdoFlags, //This is a bitmask that controls which incoming ZDO request messages are passed to the application. The bits are defined in the EmberZdoConfigurationFlags enumeration. To see if the application is required to send a ZDO response in reply to an incoming message, the application must check the APS options bitfield within the incomingMessageHandler callback to see if the EMBER_APS_OPTION_ZDO_RESPONSE_REQUIRE D flag is set.
	#[deku(id = "0x2B")]
	EzspConfigBroadcastTableSize, //The maximum number of broadcasts during a single broadcast timeout period.
	#[deku(id = "0x2C")]
	EzspConfigMacFilterTableSize, //The size of the MAC filter list table.
	#[deku(id = "0x2D")]
	EzspConfigSupportedNetworks, //The number of supported networks.
	#[deku(id = "0x2E")]
	EzspConfigSendMulticastsToSleepyAddress, //Whether multicasts are sent to the RxOnWhenIdle=true address (0xFFFD) or the sleepy broadcast address (0xFFFF). The RxOnWhenIdle=true address is the ZigBee compliant destination for multicasts.
	#[deku(id = "0x2F")]
	EzspConfigZllGroupAddresses, //ZLL group address initial configuration.
	#[deku(id = "0x30")]
	EzspConfigZllRssiThreshold, //ZLL rssi threshold initial configuration.
	#[deku(id = "0x33")]
	EzspConfigMtorrFlowControl, //Toggles the MTORR flow control in the stack.
	#[deku(id = "0x34")]
	EzspConfigRetryQueueSize, //Setting the retry queue size. Applies to all queues. Default value in the sample applications is 16.
	#[deku(id = "0x35")]
	EzspConfigNewBroadcastEntryThreshold, //Setting the new broadcast entry threshold. The number(BROADCAST_TABLE_SIZE - NEW_BROADCAST_ENTRY_THRESHOLD) of broadcast table entries are reserved for relaying the broadcast messages originated on other devices. The local device will fail to originate a broadcast message after this threshold is reached. Setting this value to BROADCAST_TABLE_SIZE and greater will effectively kill this limitation.
	#[deku(id = "0x36")]
	EzspConfigTransientKeyTimeoutS, //The length of time, in seconds, that a trust center will store a transient link key that a device can use to join its network. A transient key is added with a call to emberAddTransientLinkKey. After the transient key is added, it will be removed once this amount of time has passed. A joining device will not be able to use that key to join until it is added again on the trust center. The default value is 300 seconds, i.e., 5 minutes.
	#[deku(id = "0x37")]
	EzspConfigBroadcastMinAcksNeeded, //The number of passive acknowledgements to record from neighbors before we stop re-transmitting broadcasts
	#[deku(id = "0x38")]
	EzspConfigTcRejoinsUsingWellKnownKeyTimeoutS, //The length of time, in seconds, that a trust center will allow a Trust Center (insecure) rejoin for a device that is using the well-known link key. This timeout takes effect once rejoins using the well-known key has been allowed. This command updates the sli_zigbee_allow_tc_rejoins_using_well_known_key_timeout_sec value.
	#[deku(id = "0x39")]
	EzspConfigCtuneValue, //Valid range of a CTUNE value is 0x0000-0x01FF. Higher order bits (0xFE00) of the 16-bit value are ignored.
	#[deku(id = "0x40")]
	EzspConfigAssumeTcConcentratorType, //To configure non trust center node to assume a concentrator type of the trust center it join to, until it receive many-to-one route request from the trust center. For the trust center node, concentrator type is configured from the concentrator plugin. The stack by default assumes trust center be a low RAM concentrator that make other devices send route record to the trust center even without receiving a many-to-one route request. The default concentrator type can be changed by setting appropriate EmberAssumeTrustCenterConcentratorType config value.
	#[deku(id = "0x41")]
	EzspConfigGpProxyTableSize, //This is green power proxy table size. This value is read-only and cannot be set at runtime
	#[deku(id = "0x42")]
	EzspConfigGpSinkTableSize, //This is green power sink table size. This value is read-only and cannot be set at runtime
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum EzspStatus {
	#[deku(id = "0x00")]
	EzspSuccess, //Success.
	#[deku(id = "0x10")]
	EzspSpiErrFatal, //Fatal error.
	#[deku(id = "0x11")]
	EzspSpiErrNcpReset, //The Response frame of the current transaction indicates the NCP has reset.
	#[deku(id = "0x12")]
	EzspSpiErrOversizedEzspFrame, //The NCP is reporting that the Command frame of the current transaction is oversized (the length byte is too large).
	#[deku(id = "0x13")]
	EzspSpiErrAbortedTransaction, //The Response frame of the current transaction indicates the previous transaction was aborted (nSSEL deasserted too soon).
	#[deku(id = "0x14")]
	EzspSpiErrMissingFrameTerminator, //The Response frame of the current transaction indicates the frame terminator is missing from the Command frame.
	#[deku(id = "0x15")]
	EzspSpiErrWaitSectionTimeout, //The NCP has not provided a Response within the time limit defined by WAIT_SECTION_TIMEOUT.
	#[deku(id = "0x16")]
	EzspSpiErrNoFrameTerminator, //The Response frame from the NCP is missing the frame terminator.
	#[deku(id = "0x17")]
	EzspSpiErrEzspCommandOversized, //The Host attempted to send an oversized Command (the length byte is too large) and the AVR's spi-protocol.c blocked the transmission.
	#[deku(id = "0x18")]
	EzspSpiErrEzspResponseOversized, //The NCP attempted to send an oversized Response (the length byte is too large) and the AVR's spi-protocol.c blocked the reception.
	#[deku(id = "0x19")]
	EzspSpiWaitingForResponse, //The Host has sent the Command and is still waiting for the NCP to send a Response.
	#[deku(id = "0x1A")]
	EzspSpiErrHandshakeTimeout, //The NCP has not asserted nHOST_INT within the time limit defined by WAKE_HANDSHAKE_TIMEOUT.
	#[deku(id = "0x1B")]
	EzspSpiErrStartupTimeout, //The NCP has not asserted nHOST_INT after an NCP reset within the time limit defined by STARTUP_TIMEOUT.
	#[deku(id = "0x1C")]
	EzspSpiErrStartupFail, //The Host attempted to verify the SPI Protocol activity and version number, and the verification failed.
	#[deku(id = "0x1D")]
	EzspSpiErrUnsupportedSpiCommand, //The Host has sent a command with a SPI Byte that is unsupported by the current mode the NCP is operating in.
	#[deku(id = "0x20")]
	EzspAshInProgress, //Operation not yet complete.
	#[deku(id = "0x21")]
	EzspHostFatalError, //Fatal error detected by host.
	#[deku(id = "0x22")]
	EzspAshNcpFatalError, //Fatal error detected by NCP.
	#[deku(id = "0x23")]
	EzspDataFrameTooLong, //Tried to send DATA frame too long.
	#[deku(id = "0x24")]
	EzspDataFrameTooShort, //Tried to send DATA frame too short.
	#[deku(id = "0x25")]
	EzspNoTxSpace, //No space for tx'ed DATA frame.
	#[deku(id = "0x26")]
	EzspNoRxSpace, //No space for rec'd DATA frame.
	#[deku(id = "0x27")]
	EzspNoRxData, //No receive data available.
	#[deku(id = "0x28")]
	EzspNotConnected, //Not in Connected state.
	#[deku(id = "0x30")]
	EzspErrorVersionNotSet, //The NCP received a command before the EZSP version had been set.
	#[deku(id = "0x31")]
	EzspErrorInvalidFrameId, //The NCP received a command containing an unsupported frame ID.
	#[deku(id = "0x32")]
	EzspErrorWrongDirection, //The direction flag in the frame control field was incorrect.
	#[deku(id = "0x33")]
	EzspErrorTruncated, //The truncated flag in the frame control field was set, indicating there was not enough memory available to complete the response or that the response would have exceeded the maximum EZSP frame length.
	#[deku(id = "0x34")]
	EzspErrorOverflow, //The overflow flag in the frame control field was set, indicating one or more callbacks occurred since the previous response and there was not enough memory available to report them to the Host.
	#[deku(id = "0x35")]
	EzspErrorOutOfMemory, //Insufficient memory was available.
	#[deku(id = "0x36")]
	EzspErrorInvalidValue, //The value was out of bounds.
	#[deku(id = "0x37")]
	EzspErrorInvalidId, //The configuration id was not recognized.
	#[deku(id = "0x38")]
	EzspErrorInvalidCall, //Configuration values can no longer be modified.
	#[deku(id = "0x39")]
	EzspErrorNoResponse, //The NCP failed to respond to a command.
	#[deku(id = "0x40")]
	EzspErrorCommandTooLong, //The length of the command exceeded the maximum EZSP frame length.
	#[deku(id = "0x41")]
	EzspErrorQueueFull, //The UART receive queue was full causing a callback response to be dropped.
	#[deku(id = "0x42")]
	EzspErrorCommandFiltered, //The command has been filtered out by NCP.
	#[deku(id = "0x43")]
	EzspErrorSecurityKeyAlreadySet, //EZSP Security Key is already set
	#[deku(id = "0x44")]
	EzspErrorSecurityTypeInvalid, //EZSP Security Type is invalid
	#[deku(id = "0x45")]
	EzspErrorSecurityParametersInvalid, //EZSP Security Parameters are invalid
	#[deku(id = "0x46")]
	EzspErrorSecurityParametersAlreadySet, //EZSP Security Parameters are already set
	#[deku(id = "0x47")]
	EzspErrorSecurityKeyNotSet, //EZSP Security Key is not set
	#[deku(id = "0x48")]
	EzspErrorSecurityParametersNotSet, //EZSP Security Parameters are not set
	#[deku(id = "0x49")]
	EzspErrorUnsupportedControl, //Received frame with unsupported control byte
	#[deku(id = "0x4A")]
	EzspErrorUnsecureFrame, //Received frame is unsecure, when security is established
	#[deku(id = "0x50")]
	EzspAshErrorVersion, //Incompatible ASH version
	#[deku(id = "0x51")]
	EzspAshErrorTimeouts, //Exceeded max ACK timeouts
	#[deku(id = "0x52")]
	EzspAshErrorResetFail, //Timed out waiting for RSTACK
	#[deku(id = "0x53")]
	EzspAshErrorNcpReset, //Unexpected ncp reset
	#[deku(id = "0x54")]
	EzspErrorSerialInit, //Serial port initialization failed
	#[deku(id = "0x55")]
	EzspAshErrorNcpType, //Invalid ncp processor type
	#[deku(id = "0x56")]
	EzspAshErrorResetMethod, //Invalid ncp reset method
	#[deku(id = "0x57")]
	EzspAshErrorXonXoff, //XON/XOFF not supported by host driver
	#[deku(id = "0x70")]
	EzspAshStarted, //ASH protocol started
	#[deku(id = "0x71")]
	EzspAshConnected, //ASH protocol connected
	#[deku(id = "0x72")]
	EzspAshDisconnected, //ASH protocol disconnected
	#[deku(id = "0x73")]
	EzspAshAckTimeout, //Timer expired waiting for ack
	#[deku(id = "0x74")]
	EzspAshCancelled, //Frame in progress cancelled
	#[deku(id = "0x75")]
	EzspAshOutOfSequence, //Received frame out of sequence
	#[deku(id = "0x76")]
	EzspAshBadCrc, //Received frame with CRC error
	#[deku(id = "0x77")]
	EzspAshCommError, //Received frame with comm error
	#[deku(id = "0x78")]
	EzspAshBadAcknum, //Received frame with bad ackNum
	#[deku(id = "0x79")]
	EzspAshTooShort, //Received frame shorter than minimum
	#[deku(id = "0x7A")]
	EzspAshTooLong, //Received frame longer than maximum
	#[deku(id = "0x7B")]
	EzspAshBadControl, //Received frame with illegal control byte
	#[deku(id = "0x7C")]
	EzspAshBadLength, //Received frame with illegal length for its type
	#[deku(id = "0x7D")]
	EzspAshAckReceived, //Received ASH Ack
	#[deku(id = "0x7E")]
	EzspAshAckSent, //Sent ASH Ack
	#[deku(id = "0x7F")]
	EzspAshNakReceived, //Received ASH Nak
	#[deku(id = "0x80")]
	EzspAshNakSent, //Sent ASH Nak
	#[deku(id = "0x81")]
	EzspAshRstReceived, //Received ASH RST
	#[deku(id = "0x82")]
	EzspAshRstSent, //Sent ASH RST
	#[deku(id = "0x83")]
	EzspAshStatus, //ASH Status
	#[deku(id = "0x84")]
	EzspAshTx, //ASH TX
	#[deku(id = "0x85")]
	EzspAshRx, //ASH RX
	#[deku(id = "0x86")]
	EzspCpcErrorInit, //Failed to connect to CPC daemon or failed to open CPC endpoint
	#[deku(id = "0xFF")]
	EzspNoError, //No reset or error
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum EzspPolicyId {
	#[deku(id = "0x00")]
	EzspTrustCenterPolicy, //Controls trust center behavior.
	#[deku(id = "0x01")]
	EzspBindingModificationPolicy, //Controls how external binding modification requests are handled.
	#[deku(id = "0x02")]
	EzspUnicastRepliesPolicy, //Controls whether the Host supplies unicast replies.
	#[deku(id = "0x03")]
	EzspPollHandlerPolicy, //Controls whether pollHandler callbacks are generated.
	#[deku(id = "0x04")]
	EzspMessageContentsInCallbackPolicy, //Controls whether the message contents are included in the messageSentHandler callback.
	#[deku(id = "0x05")]
	EzspTcKeyRequestPolicy, //Controls whether the Trust Center will respond to Trust Center link key requests.
	#[deku(id = "0x06")]
	EzspAppKeyRequestPolicy, //Controls whether the Trust Center will respond to application link key requests.
	#[deku(id = "0x07")]
	EzspPacketValidateLibraryPolicy, //Controls whether ZigBee packets that appear invalid are automatically dropped by the stack. A counter will be incremented when this occurs.
	#[deku(id = "0x08")]
	EzspZllPolicy, //Controls whether the stack will process ZLL messages. 
	#[deku(id = "0x09")]
	EzspTcRejoinsUsingWellKnownKeyPolicy, //Controls whether Trust Center (insecure) rejoins for devices using the well-known link key are accepted. If rejoining using the well-known key is allowed, it is disabled again after sli_zigbee_allow_tc_rejoins_using_well_known_key_timeout_sec seconds.
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum EzspDecisionId {
	#[deku(id = "0x00")]
	EzspAllowJoins, //Send the network key in the clear to all joining and rejoining devices.
	#[deku(id = "0x01")]
	EzspAllowPreconfiguredKeyJoins, //Send the network key encrypted with the joining or rejoining device's trust center link key. The trust center and any joining or rejoining device are assumed to share a link key, either preconfigured or obtained under a previous policy. This is the default value for the  EZSP_TRUST_CENTER_POLICY.
	#[deku(id = "0x02")]
	EzspAllowRejoinsOnly, //Send the network key encrypted with the rejoining device's trust center link key. The trust center and any rejoining device are assumed to share a link key, either preconfigured or obtained under a previous policy. No new devices are allowed to join. 
	#[deku(id = "0x03")]
	EzspDisallowAllJoinsAndRejoins, //Reject all unsecured join and rejoin attempts.
	#[deku(id = "0x04")]
	EzspAllowJoinsRejoinsHaveLinkKey, //Send the network key in the clear to all joining devices. Rejoining devices are sent the network key encrypted with their trust center link key. The trust center and any rejoining device are assumed to share a link key, either preconfigured or obtained under a previous policy.
	#[deku(id = "0x05")]
	EzspIgnoreTrustCenterRejoins, //Take no action on trust center rejoin attempts.
	#[deku(id = "0x06")]
	EzspBdbJoinUsesInstallCodeKey, //Admit joins only if there is an entry in the transient key table. This corresponds to the Base Device Behavior specification where a Trust Center enforces all devices to join with an install code-derived link key.
	#[deku(id = "0x07")]
	EzspDeferJoinsRejoinsHaveLinkKey, //Delay sending the network key to a new joining device.
	#[deku(id = "0x10")]
	EzspDisallowBindingModification, //EZSP_BINDING_MODIFICATION_POLICY default decision. Do not allow the local binding table to be changed by remote nodes.
	#[deku(id = "0x11")]
	EzspAllowBindingModification, //EZSP_BINDING_MODIFICATION_POLICY decision. Allow remote nodes to change the local binding table.
	#[deku(id = "0x12")]
	EzspCheckBindingModificationsAreValidEndpointClusters, //EZSP_BINDING_MODIFICATION_POLICY decision. Allows remote nodes to set local binding entries only if the entries correspond to endpoints defined on the device, and for output clusters bound to those endpoints.
	#[deku(id = "0x20")]
	EzspHostWillNotSupplyReply, //EZSP_UNICAST_REPLIES_POLICY default decision. The NCP will automatically send an empty reply (containing no payload) for every unicast received.
	#[deku(id = "0x21")]
	EzspHostWillSupplyReply, //EZSP_UNICAST_REPLIES_POLICY decision. The NCP will only send a reply if it receives a sendReply command from the Host.
	#[deku(id = "0x30")]
	EzspPollHandlerIgnore, //EZSP_POLL_HANDLER_POLICY default decision. Do not inform the Host when a child polls.
	#[deku(id = "0x31")]
	EzspPollHandlerCallback, //EZSP_POLL_HANDLER_POLICY decision. Generate a pollHandler callback when a child polls.
	#[deku(id = "0x40")]
	EzspMessageTagOnlyInCallback, //EZSP_MESSAGE_CONTENTS_IN_CALLBACK_POLICY default decision. Include only the message tag in the messageSentHandler callback.
	#[deku(id = "0x41")]
	EzspMessageTagAndContentsInCallback, //EZSP_MESSAGE_CONTENTS_IN_CALLBACK_POLICY decision. Include both the message tag and the message contents in the messageSentHandler callback.
	#[deku(id = "0x50")]
	EzspDenyTcKeyRequests, //EZSP_TC_KEY_REQUEST_POLICY decision. When the Trust Center receives a request for a Trust Center link key, it will be ignored.
	#[deku(id = "0x51")]
	EzspAllowTcKeyRequestsAndSendCurrentKey, //EZSP_TC_KEY_REQUEST_POLICY decision. When the Trust Center receives a request for a Trust Center link key, it will reply to it with the corresponding key.
	#[deku(id = "0x52")]
	EzspAllowTcKeyRequestAndGenerateNewKey, //EZSP_TC_KEY_REQUEST_POLICY decision. When the Trust Center receives a request for a Trust Center link key, it will generate a key to send to the joiner. After generation, the key will be added to the transient key table and after verification this key will be added to the link key table.
	#[deku(id = "0x60")]
	EzspDenyAppKeyRequests, //EZSP_APP_KEY_REQUEST_POLICY decision. When the Trust Center receives a request for an application link key, it will be ignored.
	#[deku(id = "0x61")]
	EzspAllowAppKeyRequests, //EZSP_APP_KEY_REQUEST_POLICY decision. When the Trust Center receives a request for an application link key, it will randomly generate a key and send it to both partners.
	#[deku(id = "0x62")]
	EzspPacketValidateLibraryChecksEnabled, //Indicates that packet validate library checks are enabled on the NCP.
	#[deku(id = "0x63")]
	EzspPacketValidateLibraryChecksDisabled, //Indicates that packet validate library checks are NOT enabled on the NCP.
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum EmberJoinMethod {
	#[deku(id = "0x0")]
	EmberUseMacAssociation, //Normally devices use MAC Association to join a network, which respects the "permit joining" flag in the MAC Beacon. This value should be used by default.
	#[deku(id = "0x1")]
	EmberUseNwkRejoin, //For those networks where the "permit joining" flag is never turned on, they will need to use a ZigBee NWK Rejoin. This value causes the rejoin to be sent without NWK security and the Trust Center will be asked to send the NWK key to the device. The NWK key sent to the device can be encrypted with the device's corresponding Trust Center link key. That is determined by the ::EmberJoinDecision on the Trust Center returned by the ::emberTrustCenterJoinHandler().
	#[deku(id = "0x2")]
	EmberUseNwkRejoinHaveNwkKey, //For those networks where the "permit joining" flag is never turned on, they will need to use an NWK Rejoin. If those devices have been preconfigured with the NWK key (including sequence number) they can use a secured rejoin. This is only necessary for end devices since they need a parent. Routers can simply use the ::EMBER_USE_CONFIGURED_NWK_STATE join method below.
	#[deku(id = "0x3")]
	EmberUseConfiguredNwkState, //For those networks where all network and security information is known ahead of time, a router device may be commissioned such that it does not need to send any messages to begin communicating on the network.
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum EmberNodeType {
	#[deku(id = "0x00")]
	EmberUnknownDevice, //Device is not joined.
	#[deku(id = "0x01")]
	EmberCoordinator, //Will relay messages and can act as a parent to other nodes.
	#[deku(id = "0x02")]
	EmberRouter, //Will relay messages and can act as a parent to other nodes.
	#[deku(id = "0x03")]
	EmberEndDevice, //Communicates only with its parent and will not relay messages.
	#[deku(id = "0x04")]
	EmberSleepyEndDevice, //An end device whose radio can be turned off to save power. The application must poll to receive messages.
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum EzspValueId {
	#[deku(id = "0x00")]
	EzspValueTokenStackNodeData, //The contents of the node data stack token.
	#[deku(id = "0x01")]
	EzspValueMacPassthroughFlags, //The types of MAC passthrough messages that the host wishes to receive.
	#[deku(id = "0x02")]
	EzspValueEmbernetPassthroughSourceAddress, //The source address used to filter legacy EmberNet messages when the EMBER_MAC_PASSTHROUGH_EMBERNET_SOURCE flag is set in EZSP_VALUE_MAC_PASSTHROUGH_FLAGS.
	#[deku(id = "0x03")]
	EzspValueFreeBuffers, //The number of available internal RAM general purpose buffers. Read only.
	#[deku(id = "0x04")]
	EzspValueUartSynchCallbacks, //Selects sending synchronous callbacks in ezsp-uart.
	#[deku(id = "0x05")]
	EzspValueMaximumIncomingTransferSize, //The maximum incoming transfer size for the local node. Default value is set to 82 and does not use fragmentation. Sets the value in Node Descriptor. To set, this takes the input of a uint8 array of length 2 where you pass the lower byte at index 0 and upper byte at index 1.
	#[deku(id = "0x06")]
	EzspValueMaximumOutgoingTransferSize, //The maximum outgoing transfer size for the local node. Default value is set to 82 and does not use fragmentation. Sets the value in Node Descriptor. To set, this takes the input of a uint8 array of length 2 where you pass the lower byte at index 0 and upper byte at index 1.
	#[deku(id = "0x07")]
	EzspValueStackTokenWriting, //A bool indicating whether stack tokens are written to persistent storage as they change.
	#[deku(id = "0x08")]
	EzspValueStackIsPerformingRejoin, //A read-only value indicating whether the stack is currently performing a rejoin.
	#[deku(id = "0x09")]
	EzspValueMacFilterList, //A list of EmberMacFilterMatchData values.
	#[deku(id = "0x0A")]
	EzspValueExtendedSecurityBitmask, //The Ember Extended Security Bitmask.
	#[deku(id = "0x0B")]
	EzspValueNodeShortId, //The node short ID.
	#[deku(id = "0x0C")]
	EzspValueDescriptorCapability, //The descriptor capability of the local node. Write only.
	#[deku(id = "0x0D")]
	EzspValueStackDeviceRequestSequenceNumber, //The stack device request sequence number of the local node.
	#[deku(id = "0x0E")]
	EzspValueRadioHoldOff, //Enable or disable radio hold-off.
	#[deku(id = "0x0F")]
	EzspValueEndpointFlags, //The flags field associated with the endpoint data.
	#[deku(id = "0x10")]
	EzspValueMfgSecurityConfig, //Enable/disable the Mfg security config key settings.
	#[deku(id = "0x11")]
	EzspValueVersionInfo, //Retrieves the version information from the stack on the NCP.
	#[deku(id = "0x12")]
	EzspValueNextHostRejoinReason, //This will get/set the rejoin reason noted by the host for a subsequent call to emberFindAndRejoinNetwork(). After a call to emberFindAndRejoinNetwork() the host's rejoin reason will be set to EMBER_REJOIN_REASON_NONE. The NCP will store the rejoin reason used by the call to emberFindAndRejoinNetwork().Application is not required to do anything with this value. The App Framework sets this for cases of emberFindAndRejoinNetwork that it initiates, but if the app is invoking a rejoin directly, it should/can set this value to aid in debugging of any rejoin state machine issues over EZSP logs after the fact. The NCP doesn't do anything with this value other than cache it so you can read it later.
	#[deku(id = "0x13")]
	EzspValueLastRejoinReason, //This is the reason that the last rejoin took place. This value may only be retrieved, not set. The rejoin may have been initiated by the stack (NCP) or the application (host). If a host initiated a rejoin the reason will be set by default to EMBER_REJOIN_DUE_TO_APP_EVENT_1. If the application wishes to denote its own rejoin reasons it can do so by calling ezspSetValue(EMBER_VALUE_HOST_REJOIN_REASON, EMBER_REJOIN_DUE_TO_APP_EVENT_X). X is a number corresponding to one of the app events defined. If the NCP initiated a rejoin it will record this value internally for retrieval by ezspGetValue(EZSP_VALUE_REAL_REJOIN_REASON).
	#[deku(id = "0x14")]
	EzspValueNextZigbeeSequenceNumber, //The next ZigBee sequence number.
	#[deku(id = "0x15")]
	EzspValueCcaThreshold, //CCA energy detect threshold for radio.
	#[deku(id = "0x17")]
	EzspValueSetCounterThreshold, //The threshold value for a counter
	#[deku(id = "0x18")]
	EzspValueResetCounterThresholds, //Resets all counters thresholds to 0xFF
	#[deku(id = "0x19")]
	EzspValueClearCounters, //Clears all the counters
	#[deku(id = "0x1A")]
	EzspValueCertificate283K1, //The node's new certificate signed by the CA.
	#[deku(id = "0x1B")]
	EzspValuePublicKey283K1, //The Certificate Authority's public key.
	#[deku(id = "0x1C")]
	EzspValuePrivateKey283K1, //The node's new static private key.
	#[deku(id = "0x23")]
	EzspValueNwkFrameCounter, //The NWK layer security frame counter value
	#[deku(id = "0x24")]
	EzspValueApsFrameCounter, //The APS layer security frame counter value. Managed by the stack. Users should not set these unless doing backup and restore.
	#[deku(id = "0x25")]
	EzspValueRetryDeviceType, //Sets the device type to use on the next rejoin using device type
	#[deku(id = "0x29")]
	EzspValueEnableR21Behavior, //Setting this byte enables R21 behavior on the NCP.
	#[deku(id = "0x30")]
	EzspValueAntennaMode, //Configure the antenna mode(0-don't switch,1-primary,2-secondary,3-TX antenna diversity).
	#[deku(id = "0x31")]
	EzspValueEnablePta, //Enable or disable packet traffic arbitration.
	#[deku(id = "0x32")]
	EzspValuePtaOptions, //Set packet traffic arbitration configuration options.
	#[deku(id = "0x33")]
	EzspValueMfglibOptions, //Configure manufacturing library options (0-non-CSMA transmits,1-CSMA transmits). To be used with Manufacturing library.
	#[deku(id = "0x34")]
	EzspValueUseNegotiatedPowerByLpd, //Sets the flag to use either negotiated power by link power delta (LPD) or fixed power value provided by user while forming/joining a network for packet transmissions on sub-ghz interface. This is mainly for testing purposes.
	#[deku(id = "0x35")]
	EzspValuePtaPwmOptions, //Set packet traffic arbitration PWM options.
	#[deku(id = "0x36")]
	EzspValuePtaDirectionalPriorityPulseWidth, //Set packet traffic arbitration directional priority pulse width in microseconds.
	#[deku(id = "0x37")]
	EzspValuePtaPhySelectTimeout, //Set packet traffic arbitration phy select timeout(ms).
	#[deku(id = "0x38")]
	EzspValueAntennaRxMode, //Configure the RX antenna mode: (0-do not switch; 1-primary; 2-secondary; 3-RX antenna diversity).
	#[deku(id = "0x39")]
	EzspValueNwkKeyTimeout, //Configure the timeout to wait for the network key before failing a join. Acceptable timeout range [3,255]. Value is in seconds.
	#[deku(id = "0x3A")]
	EzspValueForceTxAfterFailedCcaAttempts, //The number of failed CSMA attempts due to failed CCA made by the MAC before continuing transmission with CCA disabled. This is the same as calling the emberForceTxAfterFailedCca(uint8_t csmaAttempts) API. A value of 0 disables the feature.
	#[deku(id = "0x3B")]
	EzspValueTransientKeyTimeoutS, //The length of time, in seconds, that a trust center will store a transient link key that a device can use to join its network. A transient key is added with a call to sl_zb_sec_man_import_transient_key. After the transient key is added, it will be removed once this amount of time has passed. A joining device will not be able to use that key to join until it is added again on the trust center. The default value is 300 seconds (5 minutes).
	#[deku(id = "0x3C")]
	EzspValueCoulombCounterUsage, //Cumulative energy usage metric since the last value reset of the coulomb counter plugin. Setting this value will reset the coulomb counter.
	#[deku(id = "0x3D")]
	EzspValueMaxBeaconsToStore, //When scanning, configure the maximum number of beacons to store in cache. Each beacon consumes one packet buffer in RAM.
	#[deku(id = "0x3E")]
	EzspValueEndDeviceTimeoutOptionsMask, //Set the mask to filter out unacceptable child timeout options on a router.
	#[deku(id = "0x3F")]
	EzspValueEndDeviceKeepAliveSupportMode, //The end device keep-alive mode supported by the parent.
	#[deku(id = "0x41")]
	EzspValueActiveRadioConfig, //Return the active radio config. Read only. Values are 0: Default, 1: Antenna Diversity, 2: Co-Existence, 3: Antenna diversity and Co-Existence.
	#[deku(id = "0x42")]
	EzspValueNwkOpenDuration, //Return the number of seconds the network will remain open. A return value of 0 indicates that the network is closed. Read only.
	#[deku(id = "0x43")]
	EzspValueTransientDeviceTimeout, //Timeout in milliseconds to store entries in the transient device table. If the devices are not authenticated before the timeout, the entry shall be purged
	#[deku(id = "0x44")]
	EzspValueKeyStorageVersion, //Return information about the key storage on an NCP. Returns 0 if keys are in classic key storage, and 1 if they are located in PSA key storage. Read only.
	#[deku(id = "0x45")]
	EzspValueDelayedJoinActivation, //Return activation state about TC Delayed Join on an NCP. A return value of 0 indicates that the feature is not activated.
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum Bool {
	#[deku(id = "0x00")]
	False,
	#[deku(id = "0x01")]
	True,
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum EmberIncomingMessageType {
	#[deku(id = "0x00")]
	EmberIncomingUnicast, //Unicast.
	#[deku(id = "0x01")]
	EmberIncomingUnicastReply, //Unicast reply.
	#[deku(id = "0x02")]
	EmberIncomingMulticast, //Multicast.
	#[deku(id = "0x03")]
	EmberIncomingMulticastLoopback, //Multicast sent by the local device.
	#[deku(id = "0x04")]
	EmberIncomingBroadcast, //Broadcast.
	#[deku(id = "0x05")]
	EmberIncomingBroadcastLoopback, //Broadcast sent by the local device.
	#[deku(id = "0x06")]
	EmberIncomingManyToOneRouteRequest, //Many to one route request.
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum EmberOutgoingMessageType {
	#[deku(id = "0x00")]
	EmberOutgoingDirect, //Unicast sent directly to an EmberNodeId.
	#[deku(id = "0x01")]
	EmberOutgoingViaAddressTable, //Unicast sent using an entry in the address table.
	#[deku(id = "0x02")]
	EmberOutgoingViaBinding, //Unicast sent using an entry in the binding table.
	#[deku(id = "0x03")]
	EmberOutgoingMulticast, //Multicast message. This value is passed to emberMessageSentHandler() only. It may not be passed to emberSendUnicast().
	#[deku(id = "0x04")]
	EmberOutgoingBroadcast, //Broadcast message. This value is passed to emberMessageSentHandler() only. It may not be passed to emberSendUnicast().
	#[deku(id_pat = "_")]
	Unknown(u8),
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum EmberKeyStatus {
	#[deku(id = "0x01")]
	EmberAppLinkKeyEstablished,
	#[deku(id = "0x03")]
	EmberTrustCenterLinkKeyEstablished,
	#[deku(id = "0x04")]
	EmberKeyEstablishmentTimeout,
	#[deku(id = "0x05")]
	EmberKeyTableFull,
	#[deku(id = "0x06")]
	EmberTcRespondedToKeyRequest,
	#[deku(id = "0x07")]
	EmberTcAppKeySentToRequester,
	#[deku(id = "0x08")]
	EmberTcResponseToKeyRequestFailed,
	#[deku(id = "0x09")]
	EmberTcRequestKeyTypeNotSupported,
	#[deku(id = "0x0A")]
	EmberTcNoLinkKeyForRequester,
	#[deku(id = "0x0B")]
	EmberTcRequesterEui64Unknown,
	#[deku(id = "0x0C")]
	EmberTcReceivedFirstAppKeyRequest,
	#[deku(id = "0x0D")]
	EmberTcTimeoutWaitingForSecondAppKeyRequest,
	#[deku(id = "0x0E")]
	EmberTcNonMatchingAppKeyRequestReceived,
	#[deku(id = "0x0F")]
	EmberTcFailedToSendAppKeys,
	#[deku(id = "0x10")]
	EmberTcFailedToStoreAppKeyRequest,
	#[deku(id = "0x11")]
	EmberTcRejectedAppKeyRequest
}

#[derive(Debug, DekuRead, DekuWrite, Clone)]
#[deku(id_type = "u8")]
pub enum EmberJoinDecision {
	#[deku(id = "0x00")]
	EmberUsePreconfiguredKey, //Allow the node to join. The joining node should have a pre-configured key. The security data sent to it will be encrypted with that key.
	#[deku(id = "0x01")]
	EmberSendKeyInTheClear, //Allow the node to join. Send the network key in-the-clear to the joining device.
	#[deku(id = "0x02")]
	EmberDenyJoin, //Deny join.
	#[deku(id = "0x03")]
	EmberNoAction //Take no action.
}

pub const EMBER_APS_OPTION_NONE: u16 = 0x0000; //No options.
pub const EMBER_APS_OPTION_ENCRYPTION: u16 =  0x0020; //Send the message using APS Encryption, using the Link Key shared with the destination node to encrypt the data at the APS Level.
pub const EMBER_APS_OPTION_RETRY: u16 =  0x0040; //Resend the message using the APS retry mechanism.
pub const EMBER_APS_OPTION_ENABLE_ROUTE_DISCOVERY: u16 =  0x0100; //Causes a route discovery to be initiated if no route to the destination is known.
pub const EMBER_APS_OPTION_FORCE_ROUTE_DISCOVERY: u16 =  0x0200; //Causes a route discovery to be initiated even if one is known.
pub const EMBER_APS_OPTION_SOURCE_EUI64: u16 =  0x0400; //Include the source EUI64 in the network frame.	#[deku(id = "0x01")]
pub const EMBER_APS_OPTION_DESTINATION_EUI64: u16 =  0x0800; //Include the destination EUI64 in the network frame.
pub const EMBER_APS_OPTION_ENABLE_ADDRESS_DISCOVERY: u16 =  0x1000; //Send a ZDO request to discover the node ID of the destination, if it is not already know.
pub const EMBER_APS_OPTION_POLL_RESPONSE: u16 = 0x2000; //Reserved.
pub const EMBER_APS_OPTION_ZDO_RESPONSE_REQUIRED: u16 =  0x4000; //This incoming message is a ZDO request not handled by the EmberZNet stack, and the application is responsible for sending a ZDO response. This flag is used only when the ZDO is configured to have requests handled by the application. See the EZSP_CONFIG_APPLICATION_ZDO_FLAGS configuration parameter for more information.
pub const EMBER_APS_OPTION_FRAGMENT: u16 =  0x8000; //This message is part of a fragmented message. This option may only be set for unicasts. The groupId field gives the index of this fragment in the low-order byte. If the low-order byte is zero this is the first fragment and the high-order byte contains the number of fragments in the message.

type EmberApsOption = u16; //Options to use when sending a message.
type EmberInitialSecurityBitmask = u16; //This is the Initial Security Bitmask that controls the use of various security features.
type EmberCurrentSecurityBitmask = u16; //This is the Current Security Bitmask that details the use of various security features.
type EmberDeviceUpdate = u8; //The status of the device update.

pub const EMBER_APP_RECEIVES_SUPPORTED_ZDO_REQUESTS: u8 = 0x01; // Set this flag in order to receive supported ZDO request messages via the incomingMessageHandler callback. A supported ZDO request is one that is handled by the EmberZNet stack. The stack will continue to handle the request and send the appropriate ZDO response even if this configuration option is enabled.
pub const EMBER_APP_HANDLES_UNSUPPORTED_ZDO_REQUESTS: u8 = 0x02; //Set this flag in order to receive unsupported ZDO request messages via the incomingMessageHandler callback. An unsupported ZDO request is one that is not handled by the EmberZNet stack, other than to send a 'not supported' ZDO response. If this configuration option is enabled, the stack will no longer send any ZDO response, and it is the application's responsibility to do so.
pub const EMBER_APP_HANDLES_ZDO_ENDPOINT_REQUESTS: u8 = 0x04; //Set this flag in order to receive the following ZDO request messages via the incomingMessageHandler callback: SIMPLE_DESCRIPTOR_REQUEST, MATCH_DESCRIPTORS_REQUEST, and ACTIVE_ENDPOINTS_REQUEST. If this configuration option is enabled, the stack will no longer send any ZDO response for these requests, and it is the application's responsibility to do so.
pub const EMBER_APP_HANDLES_ZDO_BINDING_REQUESTS: u8 = 0x08; //Set this flag in order to receive the following ZDO request messages via the incomingMessageHandler callback: BINDING_TABLE_REQUEST, BIND_REQUEST, and UNBIND_REQUEST. If this configuration option is enabled, the stack will no longer send any ZDO response for these requests, and it is the application's responsibility to do so.

type EmberZllKeyIndex = u8; //ZLL key encryption algorithm enumeration.
type EmberNetworkInitBitmask = u16; //Bitmask options for emberNetworkInit().

pub const EMBER_NETWORK_INIT_NO_OPTIONS: u16 = 0x0000; //No options for Network Init
pub const EMBER_NETWORK_INIT_PARENT_INFO_IN_TOKEN: u16 = 0x0001; //Save parent info (node ID and EUI64) in a token during joining/rejoin, and restore on reboot.
pub const EMBER_NETWORK_INIT_END_DEVICE_REJOIN_ON_REBOOT: u16 = 0x0002; //Send a rejoin request as an end device on reboot if parent information is persisted.

type EmberNodeId = u16; //16-bit ZigBee network address.
type EmberMulticastId = u16; //16-bit ZigBee multicast group identifier.
type EmberEUI64 = u64; //EUI 64-bit ID (an IEEE address).

//ezsp structures

//Network parameters.
#[derive(Debug, DekuRead, DekuWrite, Clone)]
pub struct EmberNetworkParameters {
	pub extended_pan_id: [u8; 8], //The network's extended PAN identifier.
	pub pan_id: u16, //The network's PAN identifier.
	pub radio_tx_power: u8, //A power setting, in dBm.
	pub radio_channel: u8, //A radio channel.
	pub join_method: EmberJoinMethod, //The method used to initially join the network.
	pub nwk_manager_id: EmberNodeId, //NWK Manager ID. The ID of the network manager in the current network. This may only be set at joining when using EMBER_USE_CONFIGURED_NWK_STATE as the join method.
	pub nwk_update_id: u8, //NWK Update ID. The value of the ZigBee nwkUpdateId known by the stack. This is used to determine the newest instance of the network after a PAN ID or channel change. This may only be set at joining when using EMBER_USE_CONFIGURED_NWK_STATE as the join method.
	pub channels: u32, //NWK channel mask. The list of preferred channels that the NWK manager has told this device to use when searching for the network. This may only be set at joining when using EMBER_USE_CONFIGURED_NWK_STATE as the join method.
}

//The parameters of a ZigBee network.
pub struct EmberZigbeeNetwork {
	pub channel: u8, //The 802.15.4 channel associated with the network.
	pub pan_id: u16, //The network's PAN identifier.
	pub extended_pan_id: [u8; 8], //The network's extended PAN identifier.
	pub allowing_join: bool, //Whether the network is allowing MAC associations.
	pub stack_profile: u8, //The Stack Profile associated with the network.
	pub nwk_update_id: u8, //The instance of the Network.
}

//ZigBee APS frame parameters.
#[derive(Debug, DekuRead, DekuWrite, Clone)]
pub struct EmberApsFrame {
	pub profile_id: u16, //The application profile ID that describes the format of the message.
	pub cluster_id: u16, //The cluster ID for this message.
	pub source_endpoint: u8, //The source endpoint.
	pub destination_endpoint: u8, //The destination endpoint.
	pub options: EmberApsOption, //A bitmask of options.
	pub group_id: u16, //The group ID for this message, if it is multicast mode.
	pub sequence: u8, //The sequence number.
}

//A multicast table entry indicates that a particular endpoint is a member of a particular multicast group. Only devices with an endpoint in a multicast group will receive messages sent to that multicast group.
#[derive(Debug, DekuRead, DekuWrite, Clone)]
pub struct EmberMulticastTableEntry {
	pub multicast_id: EmberMulticastId, //The multicast group ID.
	pub endpoint: u8, //The endpoint that is a member, or 0 if this entry is not in use (the ZDO is not a member of any multicast groups.)
	pub network_index: u8, //The network index of the network the entry is related to.
}

//A 128-bit key.
#[derive(Debug, DekuRead, DekuWrite, Clone)]
pub struct EmberKeyData {
	pub contents: [u8; 16], //The key data.
}

//The security data used to set the configuration for the stack, or the retrieved configuration currently in use.
#[derive(Debug, DekuRead, DekuWrite, Clone)]
pub struct EmberInitialSecurityState {
	pub bitmask: EmberInitialSecurityBitmask, //A bitmask indicating the security state used to indicate what the security configuration will be when the device forms or joins the network.
	pub preconfigured_key: EmberKeyData, //The pre-configured Key data that should be used when forming or joining the network. The security bitmask must be set with the EMBER_HAVE_PRECONFIGURED_KEY bit to indicate that the key contains valid data.
	pub network_key: EmberKeyData, //The Network Key that should be used by the Trust Center when it forms the network, or the Network Key currently in use by a joined device. The security bitmask must be set with EMBER_HAVE_NETWORK_KEY to indicate that the key contains valid data.
	pub network_key_sequence_number: u8, //The sequence number associated with the network key. This is only valid if the EMBER_HAVE_NETWORK_KEY has been set in the security bitmask.
	pub preconfigured_trust_center_eui64: EmberEUI64, //This is the long address of the trust center on the network that will be joined. It is usually NOT set prior to joining the network and instead it is learned during the joining message exchange. This field is only examined if EMBER_HAVE_TRUST_CENTER_EUI64 is set in the EmberInitialSecurityState::bitmask. Most devices should clear that bit and leave this field alone. This field must be set when using commissioning mode.
}

//EmberInitialSecurityBitmask
pub const EMBER_STANDARD_SECURITY_MODE:u16 = 0x0000; //This enables ZigBee Standard Security on the node.
pub const EMBER_DISTRIBUTED_TRUST_CENTER_MODE:u16 = 0x0002; //This enables Distributed Trust Center Mode for the device forming the network. (Previously known as EMBER_NO_TRUST_CENTER_MODE)
pub const EMBER_TRUST_CENTER_GLOBAL_LINK_KEY: u16 = 0x0004; //This enables a Global Link Key for the Trust Center. All nodes will share the same Trust Center Link Key.
pub const EMBER_PRECONFIGURED_NETWORK_KEY_MODE:u16 = 0x0008; //This enables devices that perform MAC Association with a pre-configured Network Key to join the network. It is only set on the Trust Center.
pub const EMBER_TRUST_CENTER_USES_HASHED_LINK_KEY:u16 = 0x0084; //This denotes that the preconfiguredKey is not the actual Link Key but a Secret Key known only to the Trust Center. It is hashed with the IEEE Address of the destination device in order to create the actual Link Key used in encryption. This is bit is only used by the Trust Center. The joining device need not set this.
pub const EMBER_HAVE_PRECONFIGURED_KEY:u16 = 0x0100; //This denotes that the preconfiguredKey element has valid data that should be used to configure the initial security state.
pub const EMBER_HAVE_NETWORK_KEY:u16 = 0x0200; //This denotes that the networkKey element has valid data that should be used to configure the initial security state.
pub const EMBER_GET_LINK_KEY_WHEN_JOINING:u16 = 0x0400; //This denotes to a joining node that it should attempt to acquire a Trust Center Link Key during joining. This is only necessary if the device does not have a pre-configured key.
pub const EMBER_REQUIRE_ENCRYPTED_KEY:u16 = 0x0800; //This denotes that a joining device should only accept an encrypted network key from the Trust Center (using its pre-configured key). A key sent in-the-clear by the Trust Center will be rejected and the join will fail. This option is only valid when utilizing a pre-configured key.
pub const EMBER_NO_FRAME_COUNTER_RESET:u16 = 0x1000; //This denotes whether the device should NOT reset its outgoing frame counters (both NWK and APS) when ::emberSetInitialSecurityState() is called. Normally it is advised to reset the frame counter before joining a new network. However in cases where a device is joining to the same network again (but not using ::emberRejoinNetwork()) it should keep the NWK and APS frame counters stored in its tokens.
pub const EMBER_GET_PRECONFIGURED_KEY_FROM_INSTALL_CODE:u16 = 0x2000; //This denotes that the device should obtain its preconfigured key from an installation code stored in the manufacturing token. The token contains a value that will be hashed to obtain the actual preconfigured key. If that token is not valid, then the call to emberSetInitialSecurityState() will fail.
pub const EMBER_HAVE_TRUST_CENTER_EUI64:u16 = 0x0040; //This denotes that the ::EmberInitialSecurityState::preconfiguredTrustCenterEui64 has a value in it containing the trust center EUI64. The device will only join a network and accept commands from a trust center with that EUI64. Normally this bit is NOT set, and the EUI64 of the trust center is learned during the join process. When commissioning a device to join onto an existing network, which is using a trust center, and without sending any messages, this bit must be set and the field ::EmberInitialSecurityState::preconfiguredTrustCenterEui64 must be populated with the appropriate EUI64.

//The security options and information currently used by the stack.
#[derive(Debug, DekuRead, DekuWrite, Clone)]
pub struct EmberCurrentSecurityState {
	bitmask: EmberCurrentSecurityBitmask, //A bitmask indicating the security options currently in use by a device joined in the network.
	trust_center_long_address: EmberEUI64, //The IEEE Address of the Trust Center device.
}

//Network Initialization parameters.
#[derive(Debug, DekuRead, DekuWrite, Clone)]
pub struct EmberNetworkInitStruct {
	pub bitmask: EmberNetworkInitBitmask, //Configuration options for network init.
}

//Describes the initial security features and requirements that will be used when forming or joining ZLL networks.
#[derive(Debug, DekuRead, DekuWrite, Clone)]
pub struct EmberZllInitialSecurityState {
	bitmask: u32, //Unused bitmask; reserved for future use.
	key_index: EmberZllKeyIndex, //The key encryption algorithm advertised by the application.
	encryption_key: EmberKeyData, //The encryption key for use by algorithms that require it.
	preconfigured_key: EmberKeyData, //The pre-configured link key used during classical ZigBee commissioning.
}

pub struct EmberGpSinkListEntry {
}

/* unused

type EzspExtendedValueId = u8; //Identifies a value based on specified characteristics. Each set of characteristics is unique to that value and is specified during the call to get the extended value.
type EzspEndpointFlags = u16; //Flags associated with the endpoint data configured on the NCP.
type EmberConfigTxPowerMode = u16; //Values for EZSP_CONFIG_TX_POWER_MODE.
type EzspDecisionBitmask = u16; //This is the policy decision bitmask that controls the trust center decision strategies. The bitmask is modified and extracted from the EzspDecisionId for supporting bitmask operations.
type EzspMfgTokenId = u8; //Manufacturing token IDs used by ezspGetMfgToken().
type EmberEventUnits = u8; //Either marks an event as inactive or specifies the units for the event execution time.
type EmberNetworkStatus = u8; //The possible join states for a node.
type EmberMacPassthroughType = u8; //MAC passthrough message type flags.
type EzspNetworkScanType = u8; //Network scan types.
type EmberCounterType = u8; //Defines the events reported to the application by the readAndClearCounters command.
type EmberZdoConfigurationFlags = u8; //Flags for controlling which incoming ZDO requests are passed to the application. To see if the application is required to send a ZDO response to an incoming message, the application must check the APS options bitfield within the incomingMessageHandler callback to see if the EMBER_APS_OPTION_ZDO_RESPONSE_REQUIRED flag is set.
type EmberConcentratorType = u16; //Type of concentrator.
type EzspZllNetworkOperation = u8; //Differentiates among ZLL network operations.
type EzspSourceRouteOverheadInformation = u8; //Validates Source Route Overhead Information cached.
type EmberMultiPhyNwkConfig = u8; //Network configuration for the desired radio interface for multi-phy network.
type EmberDutyCycleState = u8; //Duty cycle states.
type EmberRadioPowerMode = u8; //Radio power modes.
type EmberEntropySource = u8; //Entropy sources.
type SlStatus = u32; //See sl_status.h for an enumerated list.
type EmberLibraryId = u8; //A library identifier
type EmberLibraryStatus = u8; //The presence and status of the Ember library.
type EmberGpSecurityLevel = u8; //The security level of the GPD.
type EmberGpKeyType = u8; //The type of security key to use for the GPD.
type EmberBindingType = u8; //Binding types.
type EmberPanId = u16; //802.15.4 PAN ID.
type EmberKeyType = u8; //Describes the type of ZigBee security key.
type EmberZllState = u16; //ZLL device state identifier.
type SlZbSecManKeyType = u8; //Key types recognized by Zigbee Security Manager.
type SlZbSecManDerivedKeyType = u16; //Derived key types recognized by Zigbee Security Manager.
type SlZigbeeSecManFlags = u8; //Flags for key operations.
type EmberDutyCycleHectoPct = u16; //The percent of duty cycle for a limit. Duty Cycle, Limits, and Thresholds are reported in units of Percent * 100 (i.e. 10000 = 100.00%, 1 = 0.01%).
type EmberKeyStructBitmask = u16; //Describes the presence of valid data within the EmberKeyStruct structure.
type EmberGpProxyTableEntryStatus = u8; //The proxy table entry status
type EmberGpSecurityFrameCounter = u32; //The security frame counter
type EmberGpSinkTableEntryStatus = u8; //The sink table entry status

//Radio parameters.
pub struct EmberMultiPhyRadioParameters {
	radio_tx_power: i8, //A power setting, in dBm.
	radio_page: u8, //A radio page.
	radio_channel: u8, //A radio channel.
}

//An entry in the binding table.
pub struct EmberBindingTableEntry {
	r#type: EmberBindingType, //The type of binding.
	local: u8, //The endpoint on the local node.
	cluster_id: u16, //A cluster ID that matches one from the local endpoint's simple descriptor. This cluster ID is set by the provisioning application to indicate which part an endpoint's functionality is bound to this particular remote node and is used to distinguish between unicast and multicast bindings. Note that a binding can be used to send messages with any cluster ID, not just the one listed in the binding.
	remote: u8, //The endpoint on the remote node (specified by identifier).
	identifier: EmberEUI64, //A 64-bit identifier. This is either the destination EUI64 (for unicasts) or the 64-bit group address (for multicasts).
	network_index: u8, //The index of the network the binding belongs to.
}

//The implicit certificate used in CBKE.
pub struct EmberCertificateData {
	contents: [u8; 48], //The certificate data.
}

//The public key data used in CBKE.
pub struct EmberPublicKeyData {
	contents: [u8; 22], //The public key data.
}

//The private key data used in CBKE.
pub struct EmberPrivateKeyData {
	contents: [u8; 21], //The private key data.
}

//The Shared Message Authentication Code data used in CBKE.
pub struct EmberSmacData {
	contents: [u8; 16], //The Shared Message Authentication Code data.
}

//An ECDSA signature
pub struct EmberSignatureData {
	contents: [u8; 42], //The signature data.
}

//The implicit certificate used in CBKE.
pub struct EmberCertificate283k1Data {
	contents: [u8; 74], //The 283k1 certificate data.
}

//The public key data used in CBKE.
pub struct EmberPublicKey283k1Data {
	contents: [u8; 37], //The 283k1 public key data.
}

//The private key data used in CBKE.
pub struct EmberPrivateKey283k1Data {
	contents: [u8; 36], //The 283k1 private key data.
}

//An ECDSA signature
pub struct EmberSignature283k1Data {
	contents: [u8; 72], //The 283k1 signature data.
}

//The calculated digest of a message
pub struct EmberMessageDigest {
	contents: [u8; 16], //The calculated digest of a message.
}

//The hash context for an ongoing hash operation.
pub struct EmberAesMmoHashContext {
	result: [u8; 16], //The result of ongoing the hash operation.
	length: u32, //The total length of the data that has been hashed so far.
}

//Beacon data structure.
pub struct EmberBeaconData {
	channel: u8, //The channel of the received beacon.
	lqi: u8, //The LQI of the received beacon.
	rssi: i8, //The RSSI of the received beacon.
	depth: u8, //The depth of the received beacon.
	nwk_update_id: u8, //The network update ID of the received beacon.
	power: i8, //The power level of the received beacon. This field is valid only if the beacon is an enhanced beacon.
	parent_priority: i8, //The TC connectivity and long uptime from capacity field.
	pan_id: EmberPanId, //The PAN ID of the received beacon.
	extended_pan_id: [u8; 8], //The extended PAN ID of the received beacon.
	sender: EmberNodeId, //The sender of the received beacon.
	enhanced: bool, //Whether or not the beacon is enhanced.
	permit_join: bool, //Whether the beacon is advertising permit join.
	has_capacity: bool, //Whether the beacon is advertising capacity.
}

//Defines an iterator that is used to loop over cached bea-cons. Do not write to fields denoted as Private.
pub struct EmberBeaconIterator {
	beacon: EmberBeaconData, //The retrieved beacon.
	index: u8, //(Private) The index of the retrieved beacon.
}

//The parameters related to beacon prioritization.
pub struct EmberBeaconClassificationParams {
	min_rssi_for_receiving_pkts: i8, //The minimum RSSI value for receiving packets that is used in some beacon prioritization algorithms.
	beacon_classification_mask: u16, //The beacon classification mask that identifies which beacon prioritization algorithm to pick and defines the relevant parameters.
}

//A neighbor table entry stores information about the reliability of RF links to and from neighboring nodes.
pub struct EmberNeighborTableEntry {
	short_id: u16, //The neighbor's two-byte network id
	average_lqi: u8, //An exponentially weighted moving average of the link quality values of incoming packets from this neighbor as reported by the PHY.
	in_cost: u8, //The incoming cost for this neighbor, computed from the average LQI. Values range from 1 for a good link to 7 for a bad link.
	out_cost: u8, //The outgoing cost for this neighbor, obtained from the most recently received neighbor exchange message from the neighbor. A value of zero means that a neighbor exchange message from the neighbor has not been received recently enough, or that our id was not present in the most recently received one.
	age: u8, //The number of aging periods elapsed since a link status message was last received from this neighbor. The aging period is 16 seconds.
	long_id: EmberEUI64, //The 8-byte EUI64 of the neighbor.
}

//A route table entry stores information about the next hop along the route to the destination.
pub struct EmberRouteTableEntry {
	destination: u16, //The short id of the destination. A value of 0xFFFF indicates the entry is unused.
	next_hop: u16, //The short id of the next hop to this destination.
	status: u8, //Indicates whether this entry is active (0), being discovered (1), unused (3), or validating (4).
	age: u8, //The number of seconds since this route entry was last used to send a packet.
	concentrator_type: u8, //Indicates whether this destination is a High RAM Concentrator (2), a Low RAM Concentrator (1), or not a concentrator (0).
	route_record_state: u8, //For a High RAM Concentrator, indicates whether a route record is needed (2), has been sent (1), or is no long needed (0) because a source routed message from the concentrator has been received.
}

//A structure containing a key and its associated data.
pub struct EmberKeyStruct {
	bitmask: EmberKeyStructBitmask, //A bitmask indicating the presence of data within the various fields in the structure.
	r#type: EmberKeyType, //The type of the key.
	key: EmberKeyData, //The actual key data.
	outgoing_frame_counter: u32, //The outgoing frame counter associated with the key.
	incoming_frame_counter: u32, //The frame counter of the partner device associated with the key.
	sequence_number: u8, //The sequence number associated with the key.
	partner_eui64: EmberEUI64, //The IEEE address of the partner device also in possession of the key.
}

//Data associated with the ZLL security algorithm.
pub struct EmberZllSecurityAlgorithmData {
	transaction_id: u32, //Transaction identifier.
	response_id: u32, //Response identifier.
	bitmask: u16, //Bitmask.
}

//The parameters of a ZLL network.
pub struct EmberZllNetwork {
	zigbee_network: EmberZigbeeNetwork, //The parameters of a ZigBee network.
	security_algorithm: EmberZllSecurityAlgorithmData, //Data associated with the ZLL security algorithm.
	eui64: EmberEUI64, //Associated EUI64.
	node_id: EmberNodeId, //The node id.
	state: EmberZllState, //The ZLL state.
	node_type: EmberNodeType, //The node type.
	number_sub_devices: u8, //The number of sub devices.
	total_group_identifiers: u8, //The total number of group identifiers.
	rssi_correction: u8, //RSSI correction value.
}

//Information about a specific ZLL Device.
pub struct EmberZllDeviceInfoRecord {
	ieee_address: EmberEUI64, //EUI64 associated with the device.
	endpoint_id: u8, //Endpoint id.
	profile_id: u16, //Profile id.
	device_id: u16, //Device id.
	version: u8, //Associated version.
	group_id_count: u8, //Number of relevant group ids.
}

//ZLL address assignment data.
pub struct EmberZllAddressAssignment {
	node_id: EmberNodeId, //Relevant node id.
	free_node_id_min: EmberNodeId, //Minimum free node id.
	free_node_id_max: EmberNodeId, //Maximum free node id.
	group_id_min: EmberMulticastId, //Minimum group id.
	group_id_max: EmberMulticastId, //Maximum group id.
	free_group_id_min: EmberMulticastId, //Minimum free group id.
	free_group_id_max: EmberMulticastId, //Maximum free group id.
}

//Public API for ZLL stack data token.
pub struct EmberTokTypeStackZllData {
	bitmask: u32, //Token bitmask.
	free_node_id_min: u16, //Minimum free node id.
	free_node_id_max: u16, //Maximum free node id.
	my_group_id_min: u16, //Local minimum group id.
	free_group_id_min: u16, //Minimum free group id.
	free_group_id_max: u16, //Maximum free group id.
	rssi_correction: u8, //RSSI correction value.
}

//Public API for ZLL stack security token.
pub struct EmberTokTypeStackZllSecurity {
	bitmask: u32, //Token bitmask.
	key_index: u8, //Key index.
	encryption_key: [u8; 16], //Encryption key.
	preconfigured_key: [u8; 16], //Preconfigured key.
}

//A structure containing duty cycle limit configurations. All limits are absolute, and are required to be as follows: suspLimit > critThresh > limitThresh For example: suspLimit = 250 (2.5%), critThresh = 180 (1.8%), limitThresh 100 (1.00%).
pub struct EmberDutyCycleLimits {
	vendor_id: u16, //The vendor identifier field shall contain the vendor identifier of the node.
	vendor_string: [u8; 7], //The vendor string field shall contain the vendor string of the node.
}

//A structure containing per device overall duty cycle consumed (up to the suspend limit).
pub struct EmberPerDeviceDutyCycle {
	node_id: EmberNodeId, //Node Id of device whose duty cycle is reported.
	duty_cycle_consumed: EmberDutyCycleHectoPct, //Amount of overall duty cycle consumed (up to suspend limit).
}

//The transient key data structure.
pub struct EmberTransientKeyData {
	eui64: EmberEUI64, //The IEEE address paired with the transient link key.
	key_data: EmberKeyData, //The key data structure matching the transient key.
	bitmask: EmberKeyStructBitmask, //This bitmask indicates whether various fields in the structure contain valid data.
	remaining_time_seconds: u16, //The number of seconds remaining before the key is automatically timed out of the transient key table.
	network_index: u8, //The network index indicates which NWK uses this key.
}

//A structure containing a child node's data.
pub struct EmberChildData {
	eui64: EmberEUI64, //The EUI64 of the child
	r#type: EmberNodeType, //The node type of the child
	id: EmberNodeId, //The short address of the child
	phy: u8, //The phy of the child
	power: u8, //The power of the child
	timeout: u8, //The timeout of the child
	gpd_ieee_address: EmberEUI64, //The GPD's EUI64.
	source_id: u32, //The GPD's source ID.
	application_id: u8, //The GPD Application ID.
	endpoint: u8, //The GPD endpoint.
}

//A 128-bit key.
pub struct SlZbSecManKey {
	key: [u8; 16], //The key data.
}

//Context for Zigbee Security Manager operations.
pub struct SlZbSecManContext {
	core_key_type: SlZbSecManKeyType, //The type of key being referenced.
	key_index: u8, //The index of the referenced key.
	derived_type: SlZbSecManDerivedKeyType, //The type of key derivation operation to perform on a key.
	eui64: EmberEUI64, //The EUI64 associated with this key.
	multi_network_index: u8, //Multi-network index.
	flags: SlZigbeeSecManFlags, //Flag bitmask.
	psa_key_alg_permission: u32, //Algorithm to use with this key (for PSA APIs)
}

//Metadata for network keys.
pub struct SlZbSecManNetworkKeyInfo {
	network_key_set: bool, //Whether the current network key is set.
	alternate_network_key_set: bool, //Whether the alternate network key is set.
	network_key_sequence_number: u8, //Current network key sequence number.
	alt_network_key_sequence_number: u8, //Alternate network key sequence number.
	network_key_frame_counter: u32, //Frame counter for the network key.
}

//Metadata for APS link keys.
pub struct SlZbSecManApsKeyMetadata {
	bitmask: EmberKeyStructBitmask, //Bitmask of key properties
	outgoing_frame_counter: u32, //Outgoing frame counter.
	incoming_frame_counter: u32, //Incoming frame counter.
	ttl_in_seconds: u16, //Remaining lifetime (for transient keys).
}

//The internal representation of a proxy table entry
pub struct EmberGpProxyTableEntry {
	status: EmberGpProxyTableEntryStatus, //Internal status of the proxy table entry.
	options: u32, //The tunneling options (this contains both options and extendedOptions from the spec).
	gpd: EmberGpAddress, //The addressing info of the GPD.
	assigned_alias: EmberNodeId, //The assigned alias for the GPD.
	security_options: u8, //The security options field.
	gpd_security_frame_counter: EmberGpSecurityFrameCounter, //The security frame counter of the GPD.
	gpd_key: EmberKeyData, //The key to use for GPD.
	sink_list: [EmberGpSinkListEntry; 2], //The list of sinks (hardcoded to 2 which is the spec minimum).
	groupcast_radius: u8, //The groupcast radius.
	search_counter: u8, //The search counter.
}

//A GP address structure.
pub struct EmberGpAddress {
	id: [u8; 8], //Contains either a 4-byte source ID or an 8-byte IEEE address, as indicated by the value of the applicationId field.
	application_id: u8, //The GPD Application ID specifying either source ID (0x00) or IEEE address (0x02).
	endpoint: u8, //The GPD endpoint.
}

/* todo
	r#type: EmberGpSinkType,
	target: 
  union {
    EmberGpSinkAddress unicast;
    EmberGpSinkGroup groupcast;
    EmberGpSinkGroup groupList;   // Entry for Sink Group List
  } target;
} EmberGpSinkListEntry;
*/

//The internal representation of a sink table entry.
pub struct EmberGpSinkTableEntry {
	status: EmberGpSinkTableEntryStatus, //Internal status of the sink table entry.
	options: u32, //The tunneling options (this contains both options and extendedOptions from the spec).
	gpd: EmberGpAddress, //The addressing info of the GPD.
	device_id: u8, //The device id for the GPD.
	sink_list: [EmberGpSinkListEntry; 2], //The list of sinks (hardcoded to 2 which is the spec minimum).
	assigned_alias: EmberNodeId, //The assigned alias for the GPD.
	groupcast_radius: u8, //The groupcast radius.
	security_options: u8, //The security options field.
	gpd_security_frame_counter: EmberGpSecurityFrameCounter, //The security frame counter of the GPD.
	gpd_key: EmberKeyData, //The key to use for GPD.
}

//Information of a token in the token table.
pub struct EmberTokenInfo {
	nvm3_key: u32, //NVM3 key of the token
	is_cnt: bool, //Token is a counter type
	is_idx: bool, //Token is an indexed token
	size: u8, //Size of the token
	array_size: u8, //Array size of the token
}

//Token Data
pub struct EmberTokenData {
	size: u32, //Token data size in bytes
	data: [u8; 64], //Token data pointer
}

*/

