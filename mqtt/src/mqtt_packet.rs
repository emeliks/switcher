use core::{ fmt::{ self, Display, Formatter }, str };

#[derive(Debug)]
pub enum Error {
	BadMqttHeader,
	MalformedRemainingLength,
	PacketTooShort,
	NotImplemented(String),
	PacketTooLong,
	BuffTooShort,
	BuffTooLong,
	Utf8Error(str::Utf8Error),
	BadPropertyNum(u8),
}

impl Display for Error {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self {
			Self::BadMqttHeader => write!(f, "Bad MQTT header"),
			Self::MalformedRemainingLength => write!(f, "Malformed remaining length"),
			Self::PacketTooShort => write!(f, "Packet too short"),
			Self::NotImplemented(s) => write!(f, "Not implemented: {}", s),
			Self::PacketTooLong => write!(f, "Packet too long"),
			Self::BuffTooShort => write!(f, "Buffer too short"),
			Self::BuffTooLong => write!(f, "Buffer too long"),
			Self::Utf8Error(s) => write!(f, "{}", s),
			Self::BadPropertyNum(n) => write!(f, "Bad property number {}", n),
		}
	}
}

impl From<str::Utf8Error> for Error {
	fn from(r: str::Utf8Error) -> Self {
		Error::Utf8Error(r)
	}
}

impl From<Error> for frames::Error {
	fn from(r: Error) -> Self {
		frames::Error::Other(r.to_string())
	}
}

impl frames::Frame for MqttPacket {
	type Params = u8;

	fn get_buffer_len(buf: &mut Vec<u8>, _params: &Self::Params) -> Result<usize, frames::Error> {
		Ok(MqttPacket::get_buffer_len(&buf)?)
	}

	fn from_buf(buf: &[u8], params: &Self::Params) -> Result<Self, frames::Error> {
		Ok(MqttPacket::from_buf(buf, *params)?)
	}

	fn as_bytes(&self, params: &Self::Params, buf: &mut Vec<u8>) -> Result<(), frames::Error> {
		Ok(MqttPacket::as_bytes(&self, *params, buf)?)
	}
}

//mqtt packet

#[derive(Default, Debug)]
pub struct ConnectProps {
	pub protocol_name: String,
	pub protocol_level: u8,
	pub keep_alive: u16,
	pub will_retain: bool,
	pub will_qos: u8,
	pub clean_session: bool,
	pub client_identifier: String,
	pub will_topic: Option<String>,
	pub will_message: Option<String>,
	pub user_name: Option<String>,
	pub password: Option<String>,
	//mqtt 5.0
	pub properties: Option<Vec<MqttProperty>>,
	pub will_properties: Option<Vec<MqttProperty>>
}

#[derive(Default, Debug)]
pub struct PublishProps {
	pub dup: bool,
	pub qos: u8,
	pub retain: bool,
	pub topic_name: String,
	pub packet_identifier: u16,
	pub payload: Vec<u8>,
	//mqtt 5.0
	pub properties: Option<Vec<MqttProperty>>
}

#[derive(Debug)]
pub enum ConnackReturnCode {
	Accepted,
	UnacceptableProtocolVersion,
	IdentifierRejected,
	ServerUnavailable,
	BadUsernameOrPassword,
	NotAuthorized,
	//mqtt 5.0
	Error(u8)
}

impl Default for ConnackReturnCode {
	fn default() -> ConnackReturnCode {
		ConnackReturnCode::Accepted
	}
}

#[derive(Default, Debug)]
pub struct ConnackProps {
	pub session_present: bool,
	pub return_code: ConnackReturnCode,
	//mqtt 5.0
	pub properties: Option<Vec<MqttProperty>>
}

#[derive(Default, Debug, Clone)]
pub struct SubscribeTopic {
	pub topic_name: String,
	pub req_qos: u8,

	//mqtt 5.0
	pub retain_handling: u8,
	pub rap: bool,
	pub nl: bool,
}

#[derive(Default, Debug)]
pub struct SubscribeProps {
	pub packet_identifier: u16,
	pub topics: Vec<SubscribeTopic>,

	//mqtt 5.0
	pub properties: Option<Vec<MqttProperty>>
}

#[derive(Debug)]
pub struct UnsubscribeProps {
	pub packet_identifier: u16,
	pub topics: Vec<String>,

	//mqtt 5.0
	pub properties: Option<Vec<MqttProperty>>
}

#[derive(Default, Debug)]
pub struct SubackProps {
	pub packet_identifier: u16,
	pub topics: Vec<SubackRet>,

	//mqtt 5.0
	pub properties: Option<Vec<MqttProperty>>
}

#[derive(Debug)]
pub enum SubackRet {
	Success(u8),
	Failure(u8)
}

#[derive(Default, Debug)]
pub struct PubackProps {
	pub packet_identifier: u16,

	//mqtt 5.0
	pub reason_code: u8,
	pub properties: Option<Vec<MqttProperty>>
}

#[derive(Default, Debug)]
pub struct PubrecProps {
	pub packet_identifier: u16,

	//mqtt 5.0
	pub reason_code: u8,
	pub properties: Option<Vec<MqttProperty>>
}

#[derive(Default, Debug)]
pub struct PubrelProps {
	pub packet_identifier: u16,

	//mqtt 5.0
	pub reason_code: u8,
	pub properties: Option<Vec<MqttProperty>>
}

#[derive(Default, Debug)]
pub struct PubcompProps {
	pub packet_identifier: u16,

	//mqtt 5.0
	pub reason_code: u8,
	pub properties: Option<Vec<MqttProperty>>
}

#[derive(Default, Debug)]
pub struct UnsubackProps {
	pub packet_identifier: u16,

	//mqtt 5.0
	pub reason_code: u8,
	pub properties: Option<Vec<MqttProperty>>
}

#[derive(Debug)]
pub struct DisconnectProps {
	//mqtt 5.0
	pub reason_code: u8,
	pub properties: Option<Vec<MqttProperty>>
}

#[derive(Debug)]
pub enum MqttPacket {
	Connect(ConnectProps),
	Connack(ConnackProps),
	Publish(PublishProps),
	Puback(PubackProps),
	Pubrec(PubrecProps),
	Pubrel(PubrelProps),
	Pubcomp(PubcompProps),
	Subscribe(SubscribeProps),
	Suback(SubackProps),
	Unsubscribe(UnsubscribeProps),
	Unsuback(UnsubackProps),
	Pingreq,
	Pingresp,
	Disconnect(DisconnectProps),
	Reserved
}

#[derive(Debug)]
pub enum MqttProperty {
	PayloadFormatIndicator(u8),
	MessageExpiryInterval(u32),
	ContentType(String),
	ResponseTopic(String),
	CorrelationData(Vec<u8>),
	SubscriptionIdentifier(u32),
	SessionExpiryInterval(u32),
	AssignedClientIdentifier(String),
	ServerKeepAlive(u16),
	AuthenticationMethod(String),
	AuthenticationData(Vec<u8>),
	RequestProblemInformation(u8),
	WillDelayInterval(u16),
	RequestResponseInformation(u8),
	ResponseInformation(String),
	ServerReference(String),
	ReasonString(String),
	ReceiveMaximum(u16),
	TopicAliasMaximum(u16),
	TopicAlias(u16),
	MaximumQoS(u8),
	RetainAvailable(u8),
	UserProperty(String, String),
	MaximumPacketSize(u32),
	WildcardSubscriptionAvailable(u8),
	SubscriptionIdentifierAvailable(u8),
	SharedSubscriptionAvailable(u8)
}

impl MqttProperty {
	pub fn get_prop_type(&self) -> u8 {
		match self {
			MqttProperty::PayloadFormatIndicator(_) => 1,
			MqttProperty::MessageExpiryInterval(_) => 2,
			MqttProperty::ContentType(_) => 3,
			MqttProperty::ResponseTopic(_) => 8,
			MqttProperty::CorrelationData(_) => 9,
			MqttProperty::SubscriptionIdentifier(_) => 11,
			MqttProperty::SessionExpiryInterval(_) => 17,
			MqttProperty::AssignedClientIdentifier(_) => 18,
			MqttProperty::ServerKeepAlive(_) => 19,
			MqttProperty::AuthenticationMethod(_) => 21,
			MqttProperty::AuthenticationData(_) => 22,
			MqttProperty::RequestProblemInformation(_) => 23,
			MqttProperty::WillDelayInterval(_) => 24,
			MqttProperty::RequestResponseInformation(_) => 25,
			MqttProperty::ResponseInformation(_) => 26,
			MqttProperty::ServerReference(_) => 28,
			MqttProperty::ReasonString(_) => 31,
			MqttProperty::ReceiveMaximum(_) => 33,
			MqttProperty::TopicAliasMaximum(_) => 34,
			MqttProperty::TopicAlias(_) => 35,
			MqttProperty::MaximumQoS(_) => 36,
			MqttProperty::RetainAvailable(_) => 37,
			MqttProperty::UserProperty(_, _) => 38,
			MqttProperty::MaximumPacketSize(_) => 39,
			MqttProperty::WildcardSubscriptionAvailable(_) => 40,
			MqttProperty::SubscriptionIdentifierAvailable(_) => 41,
			MqttProperty::SharedSubscriptionAvailable(_) => 42
		}
	}
}

pub const MAX_PROTOCOL_LEVEL: u8 = 5;

impl MqttPacket {
	pub fn get_buffer_len(buf: &[u8]) -> Result<usize, Error> {
		let buf_len = buf.len();

		if buf_len == 0 {
			//header
			return Ok(1);
		}
		else {
			let mut pos = 1;

			loop {
				//vbi
				if buf_len <= pos {
					return Ok(1);
				}

				if buf[pos] & 0x80 == 0 {
					break;
				}

				pos += 1;
			}

			let mut packet_len = pos + 1;

			let mut pos = 1;
			let vbi = read_vbi(&buf, &mut pos)?;

			packet_len += vbi as usize;

			if packet_len >= buf_len {
				Ok(packet_len - buf_len)
			}
			else {
				Err(Error::BuffTooLong)
			}
		}
	}

	pub fn from_buf(buf: &[u8], protocol_level: u8) -> Result<MqttPacket, Error> {
		let h = buf[0];

		let mut pos = 1;
		_ = read_vbi(buf, &mut pos);

		//packet type
		let b_pt = get_packet_type(buf);

		//skip header and length bytes
		let buf = &buf[pos..];

		//flags
		let b_f = h & 0x0f;

		if b_pt == 0 || b_pt == 15 {
			return Err(Error::BadMqttHeader);
		}

		match b_pt {
			6 | 8 | 10 => {
				if b_f != 0b0010 {
					return Err(Error::BadMqttHeader);
				}
			},
			1 | 2 | 4 | 5 | 7 | 9 | 11 | 12 | 13 | 14 => {
				if b_f != 0 {
					return Err(Error::BadMqttHeader);
				}
			},
			_ => {}
		}

		let mut packet = create_packet(b_pt, b_f);

		if buf.len() > 0 {
			let mut pos: usize = 0;

			//variable header
			match &mut packet {
				MqttPacket::Connect(cp) => {
					read_str(buf, &mut pos, &mut cp.protocol_name)?;

					cp.protocol_level = read_u8(buf, &mut pos)?;
					let connect_flags = read_u8(buf, &mut pos)?;

					cp.will_retain = connect_flags & 0b00100000 != 0;
					cp.will_qos = (connect_flags >> 3) & 0b11;
					cp.clean_session = connect_flags & 0b00000010 != 0;
					cp.keep_alive = read_u16(buf, &mut pos)?;

					if cp.protocol_level == 5 {
						cp.properties = read_properties(buf, &mut pos)?;
					}

					//string fields
					read_str(buf, &mut pos, &mut cp.client_identifier)?;

					if connect_flags & 0b00000100 != 0 {
						if cp.protocol_level == 5 {
							cp.will_properties = read_properties(buf, &mut pos)?;
						}

						cp.will_topic = Some(read_str_r(buf, &mut pos)?);
						cp.will_message = Some(read_str_r(buf, &mut pos)?);
					}

					if connect_flags & 0b10000000 != 0 {
						cp.user_name = Some(read_str_r(buf, &mut pos)?);
					}

					if connect_flags & 0b01000000 != 0 {
						cp.password = Some(read_str_r(buf, &mut pos)?);
					}
				},
				MqttPacket::Connack(cp) => {
					let b = read_u8(buf, &mut pos)?;

					cp.session_present = b & 1 == 1;
					cp.return_code = match read_u8(buf, &mut pos)? {
						0 => ConnackReturnCode::Accepted,
						//mqtt 3.1.1
						1 => ConnackReturnCode::UnacceptableProtocolVersion,
						2 => ConnackReturnCode::IdentifierRejected,
						3 => ConnackReturnCode::ServerUnavailable,
						4 => ConnackReturnCode::BadUsernameOrPassword,
						5 => ConnackReturnCode::NotAuthorized,
						//mqtt 5.0
						n => ConnackReturnCode::Error(n)
					};

					if protocol_level == 5 {
						cp.properties = read_properties(buf, &mut pos)?;
					}
				},
				MqttPacket::Subscribe(sp) => {
					sp.packet_identifier = read_u16(buf, &mut pos)?;

					if protocol_level == 5 {
						sp.properties = read_properties(buf, &mut pos)?;
					}

					while buf.len() - pos > 0 {
						let topic_name = read_str_r(buf, &mut pos)?;
						let so = read_u8(buf, &mut pos)?;

						sp.topics.push(SubscribeTopic {
							topic_name,
							req_qos: so & 0b11,

							//mqtt 5.0
							retain_handling: (so & 0b110000) >> 4,
							rap: so & 0b1000 == 0b1000,
							nl: so & 0b100 == 0b100
						});
					}
				},
				MqttPacket::Unsubscribe(usp) => {
					if protocol_level == 5 {
						usp.properties = read_properties(buf, &mut pos)?;
					}

					if protocol_level < 5 {
						//todo check
						usp.packet_identifier = read_u16(buf, &mut pos)?;
					}

					while buf.len() - pos > 0 {
						usp.topics.push(read_str_r(buf, &mut pos)?);
					}
				},
				MqttPacket::Suback(sp) => {
					if protocol_level == 5 {
						sp.properties = read_properties(buf, &mut pos)?;
					}

					sp.packet_identifier = read_u16(buf, &mut pos)?;

					while buf.len() - pos > 0 {
						let b = read_u8(buf, &mut pos)?;
						sp.topics.push(if b < 0x80 {
							SubackRet::Success(b)
						}
						else {
							SubackRet::Failure(b)
						});
					}
				},
				MqttPacket::Unsuback(usp) => {
					usp.packet_identifier = read_u16(buf, &mut pos)?;

					if protocol_level == 5 {
						usp.properties = read_properties(buf, &mut pos)?;
						usp.reason_code = read_u8(buf, &mut pos)?;
					}
				},
				MqttPacket::Publish(pp) => {
					pp.topic_name = read_str_r(buf, &mut pos)?;

					if pp.qos == 1 || pp.qos == 2 {
						pp.packet_identifier = read_u16(buf, &mut pos)?;
					}

					if protocol_level == 5 {
						pp.properties = read_properties(buf, &mut pos)?;
					}

					for b in buf[pos..].iter() {
						pp.payload.push(*b)
					}
				},
				MqttPacket::Puback(pp) => {
					pp.packet_identifier = read_u16(buf, &mut pos)?;

					if protocol_level == 5 {
						pp.reason_code = read_u8(buf, &mut pos)?;
						pp.properties = read_properties(buf, &mut pos)?;
					}
				},
				MqttPacket::Pubrec(pp) => {
					pp.packet_identifier = read_u16(buf, &mut pos)?;

					if protocol_level == 5 {
						pp.reason_code = read_u8(buf, &mut pos)?;
						pp.properties = read_properties(buf, &mut pos)?;
					}
				},
				MqttPacket::Pubrel(pp) => {
					pp.packet_identifier = read_u16(buf, &mut pos)?;

					if protocol_level == 5 {
						pp.reason_code = read_u8(buf, &mut pos)?;
						pp.properties = read_properties(buf, &mut pos)?;
					}
				},
				MqttPacket::Pubcomp(pp) => {
					pp.packet_identifier = read_u16(buf, &mut pos)?;

					if protocol_level == 5 {
						pp.reason_code = read_u8(buf, &mut pos)?;
						pp.properties = read_properties(buf, &mut pos)?;
					}
				},
				MqttPacket::Disconnect(dp) => {
					if protocol_level == 5 {
						dp.reason_code = read_u8(buf, &mut pos)?;
						dp.properties = read_properties(buf, &mut pos)?;
					}
				}
				_ => {}
			}
		}

		Ok(packet)
	}

	pub fn as_bytes(&self, protocol_level: u8, buf: &mut Vec<u8>) -> Result<(), Error> {

		//flags
		let mut b_f = 0;

		//remaining length
		let mut rem_len = 0;

		//packet type
		let b_pt = match self {
			MqttPacket::Connect(cp) => {
				rem_len += 2;
				rem_len += cp.protocol_name.as_bytes().len();

				//protocol_level + connect_flags + keep_alive
				rem_len += 4;

				rem_len += 2;
				rem_len += cp.client_identifier.as_bytes().len();

				let will = cp.will_topic != None || cp.will_message != None;

				if will {
					rem_len += 2;

					if let Some(will_topic) = &cp.will_topic {
						rem_len += will_topic.as_bytes().len();
					}

					rem_len += 2;

					if let Some(will_message) = &cp.will_message {
						rem_len += will_message.as_bytes().len();
					}
				}

				if let Some(user_name) = &cp.user_name {
					rem_len += 2;
					rem_len += user_name.as_bytes().len();
				}

				if let Some(password) = &cp.password {
					rem_len += 2;
					rem_len += password.as_bytes().len();
				}

				if cp.protocol_level == 5 {
					let pl = get_properties_len(&cp.properties);

					rem_len += vbi_len(pl) as usize;
					rem_len += pl as usize;

					if will {
						let pl = get_properties_len(&cp.will_properties);

						rem_len += vbi_len(pl) as usize;
						rem_len += pl as usize;
					}
				}

				1
			},
			MqttPacket::Connack(cp) => {
				rem_len += 2;

				if protocol_level == 5 {
					let pl = get_properties_len(&cp.properties);

					rem_len += vbi_len(pl) as usize;
					rem_len += pl as usize;
				}

				2
			},
			MqttPacket::Publish(pp) => {
				b_f =
					if pp.dup { 8 } else { 0 } +
					(pp.qos << 1) +
					if pp.retain { 1 } else { 0 };

				rem_len += 2;
				rem_len += pp.topic_name.as_bytes().len();

				if pp.qos > 0 {
					rem_len += 2;
				}

				rem_len += pp.payload.len();

				if protocol_level == 5 {
					let pl = get_properties_len(&pp.properties);

					rem_len += vbi_len(pl) as usize;
					rem_len += pl as usize;
				}

				3
			},
			MqttPacket::Puback(pp) => {
				rem_len += 2;

				if protocol_level == 5 {
					rem_len += 1;

					let pl = get_properties_len(&pp.properties);

					rem_len += vbi_len(pl) as usize;
					rem_len += pl as usize;
				}

				4
			},
			MqttPacket::Pubrec(pp) => {
				rem_len += 2;

				if protocol_level == 5 {
					rem_len += 1;

					let pl = get_properties_len(&pp.properties);

					rem_len += vbi_len(pl) as usize;
					rem_len += pl as usize;
				}

				5
			},
			MqttPacket::Pubrel(pp) => {
				//Bits 3,2,1 and 0 of the fixed header in the PUBREL Control Packet are reserved and MUST be set to 0,0,1 and 0 respectively. The Server MUST treat any other value as malformed and close the Network Connection [MQTT-3.6.1-1].
				b_f = 0b0010;

				rem_len += 2;

				if protocol_level == 5 {
					rem_len += 1;

					let pl = get_properties_len(&pp.properties);

					rem_len += vbi_len(pl) as usize;
					rem_len += pl as usize;
				}

				6
			},
			MqttPacket::Pubcomp(pp) => {
				rem_len += 2;

				if protocol_level == 5 {
					rem_len += 1;

					let pl = get_properties_len(&pp.properties);

					rem_len += vbi_len(pl) as usize;
					rem_len += pl as usize;
				}

				7
			},
			MqttPacket::Subscribe(sp) => {
				rem_len += 2;

				//Bits 3,2,1 and 0 of the fixed header of the SUBSCRIBE Control Packet are reserved and MUST be set to 0,0,1 and 0 respectively. The Server MUST treat any other value as malformed and close the Network Connection [MQTT-3.8.1-1
				b_f = 0b0010;

				for st in &sp.topics {
					//2 bytes len + req qos
					rem_len += 3 + st.topic_name.as_bytes().len();
				}

				if protocol_level == 5 {
					let pl = get_properties_len(&sp.properties);

					rem_len += vbi_len(pl) as usize;
					rem_len += pl as usize;
				}

				8
			},
			MqttPacket::Suback(sp) => {
				rem_len += 2 + sp.topics.len();

				if protocol_level == 5 {
					let pl = get_properties_len(&sp.properties);

					rem_len += vbi_len(pl) as usize;
					rem_len += pl as usize;
				}

				9
			},
			MqttPacket::Unsubscribe(up) => {
				//Bits 3,2,1 and 0 of the fixed header of the UNSUBSCRIBE Control Packet are reserved and MUST be set to 0,0,1 and 0 respectively. The Server MUST treat any other value as malformed and close the Network Connection [MQTT-3.10.1-1].
				b_f = 0b0010;

				rem_len += 2;

				for topic_name in &up.topics {
					//2 bytes len
					rem_len += 2 + topic_name.as_bytes().len();
				}

				if protocol_level == 5 {
					let pl = get_properties_len(&up.properties);

					rem_len += vbi_len(pl) as usize;
					rem_len += pl as usize;
				}

				10
			},
			MqttPacket::Unsuback(up) => {
				rem_len += 2;

				if protocol_level == 5 {
					rem_len += 1;

					let pl = get_properties_len(&up.properties);

					rem_len += vbi_len(pl) as usize;
					rem_len += pl as usize;
				}

				11
			},
			MqttPacket::Pingreq => 12,
			MqttPacket::Pingresp => 13,
			MqttPacket::Disconnect(dp) => {
				if protocol_level == 5 {
					rem_len += 1;

					let pl = get_properties_len(&dp.properties);

					rem_len += vbi_len(pl) as usize;
					rem_len += pl as usize;
				}

				14
			},
			MqttPacket::Reserved => 0
		};

		//header
		buf.push((b_pt << 4) + b_f);

		//remaining length
		push_vbi(buf, rem_len as u32);

		//content
		match self {
			MqttPacket::Connect(cp) => {
				push_str(buf, &cp.protocol_name);

				buf.push(cp.protocol_level);
				let will = cp.will_topic != None || cp.will_message != None;

				let connect_flags: u8 = 
					if cp.will_retain { 0b00100000 } else { 0 } +
					(cp.will_qos & 0b11) << 3 +
					if cp.clean_session { 0b00000010 } else { 0 } +
					if cp.user_name != None { 0b10000000 } else { 0 } +
					if cp.password != None { 0b01000000 } else { 0 } +
					if will { 0b00000100 } else { 0 };

				buf.push(connect_flags);
				push_u16(buf, cp.keep_alive);

				if cp.protocol_level == 5 {
					push_properties(buf, &cp.properties);
				}

				//string fields
				push_str(buf, &cp.client_identifier);

				if will {
					if cp.protocol_level == 5 {
						push_properties(buf, &cp.will_properties);
					}

					push_str(buf, cp.will_topic.as_deref().unwrap_or(""));
					push_str(buf, cp.will_message.as_deref().unwrap_or(""));
				}

				if let Some(user_name) = &cp.user_name {
					push_str(buf, user_name);
				}

				if let Some(password) = &cp.password {
					push_str(buf, password);
				}
			},
			MqttPacket::Connack(cp) => {
				buf.push(if cp.session_present { 1 } else { 0 });
				buf.push(match cp.return_code {
					ConnackReturnCode::Accepted => 0,
					//mqtt 3.1.1
					ConnackReturnCode::UnacceptableProtocolVersion => 1,
					ConnackReturnCode::IdentifierRejected => 2,
					ConnackReturnCode::ServerUnavailable => 3,
					ConnackReturnCode::BadUsernameOrPassword => 4,
					ConnackReturnCode::NotAuthorized => 5,
					//mqtt 5.0
					ConnackReturnCode::Error(n) => n
				});

				if protocol_level == 5 {
					push_properties(buf, &cp.properties);
				}
			},
			MqttPacket::Subscribe(sp) => {
				push_u16(buf, sp.packet_identifier);

				if protocol_level == 5 {
					push_properties(buf, &sp.properties);
				}

				for st in &sp.topics {
					push_str(buf, &st.topic_name);
					buf.push(st.req_qos);
				}
			},
			MqttPacket::Suback(sp) => {
				if protocol_level == 5 {
					push_properties(buf, &sp.properties);
				}

				push_u16(buf, sp.packet_identifier);

				for pr in &sp.topics {
					buf.push(match pr {
						SubackRet::Success(qos) => *qos,
						SubackRet::Failure(err) => *err
					});
				}
			},
			MqttPacket::Publish(pp) => {
				push_str(buf, &pp.topic_name);

				if pp.qos == 1 || pp.qos == 2 {
					push_u16(buf, pp.packet_identifier);
				}

				if protocol_level == 5 {
					push_properties(buf, &pp.properties);
				}

				for b in &pp.payload {
					buf.push(*b);
				}
			},
			MqttPacket::Puback(sp) => {
				push_u16(buf, sp.packet_identifier);

				if protocol_level == 5 {
					push_u8(buf, sp.reason_code);
					push_properties(buf, &sp.properties);
				}
			},
			MqttPacket::Unsuback(up) => {
				push_u16(buf, up.packet_identifier);

				if protocol_level == 5 {
					push_properties(buf, &up.properties);
					push_u8(buf, up.reason_code);
				}
			},
			MqttPacket::Pingreq => {},
			MqttPacket::Pingresp => {},
			MqttPacket::Disconnect(dp) => {
				if protocol_level == 5 {
					push_u8(buf, dp.reason_code);
					push_properties(buf, &dp.properties);
				}
			},
			_ => {
				return Err(Error::NotImplemented(format!("Not implemented packet type {}", b_pt)));
			}
		};

		Ok(())
	}
}

fn read_u8(buf: &[u8], pos: &mut usize) -> Result<u8, Error> {
	if buf.len() > *pos {
		let ret = buf[*pos];

		*pos += 1;
		Ok(ret)
	}
	else {
		Err(Error::BuffTooShort)
	}
}

fn read_u16(buf: &[u8], pos: &mut usize) -> Result<u16, Error> {
	if buf.len() > *pos + 1 {
		let ret = ((buf[*pos] as u16) << 8) + buf[*pos + 1] as u16;

		*pos += 2;
		Ok(ret)
	}
	else {
		Err(Error::BuffTooShort)
	}
}

fn read_u32(buf: &[u8], pos: &mut usize) -> Result<u32, Error> {
	if buf.len() > *pos + 3 {
		let ret = ((buf[*pos] as u32) << 24) + ((buf[*pos + 1] as u32) << 16) + ((buf[*pos + 2] as u32) << 8) + buf[*pos + 3] as u32;

		*pos += 4;
		Ok(ret)
	}
	else {
		Err(Error::BuffTooShort)
	}
}

fn read_str(buf: &[u8], pos: &mut usize, s: &mut String) -> Result<(), Error> {
	if buf.len() < *pos + 2 {
		return Err(Error::PacketTooShort);
	}

	let s_len = read_u16(buf, pos)? as usize;

	if buf.len() < *pos + s_len {
		return Err(Error::PacketTooShort);
	}

	s.push_str(str::from_utf8(&buf[*pos..][..s_len])?);
	*pos += s_len;

	Ok(())
}

fn read_binary_data(buf: &[u8], pos: &mut usize) -> Result<Vec<u8>, Error> {
	if buf.len() < *pos + 2 {
		return Err(Error::PacketTooShort);
	}

	let len = read_u16(buf, pos)? as usize;

	if buf.len() < *pos + len {
		return Err(Error::PacketTooShort);
	}

	*pos += len;

	Ok(buf[*pos..][..len].to_vec())
}

fn read_str_r(buf: &[u8], pos: &mut usize) -> Result<String, Error> {
	let mut s = String::new();
	read_str(buf, pos, &mut s)?;

	Ok(s)
}

fn push_str(buf: &mut Vec<u8>, s: &str) {
	let str_bytes = s.as_bytes();
	let str_bytes_len = str_bytes.len() as u16;

	let b = str_bytes_len.to_be_bytes();
	buf.push(b[0]);
	buf.push(b[1]);

	for b in str_bytes {
		buf.push(*b);
	}
}

fn push_binary_data(buf: &mut Vec<u8>, v: &Vec<u8>) {
	buf.extend(v);
}

fn push_u8(buf: &mut Vec<u8>, v: u8) {
	buf.push(v);
}

fn push_u16(buf: &mut Vec<u8>, v: u16) {
	let b = v.to_be_bytes();
	buf.push(b[0]);
	buf.push(b[1]);
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
	let b = v.to_be_bytes();
	buf.push(b[0]);
	buf.push(b[1]);
	buf.push(b[2]);
	buf.push(b[3]);
}

fn push_vbi(buf: &mut Vec<u8>, v: u32) {
	let mut v = v;

	loop {
		let mut eb: u8 = (v % 128) as u8;
		v = v >> 7;

		if v > 0 {
			eb = eb | 128;
		}

		buf.push(eb);

		if v == 0 {
			break;
		}
	}
}

fn vbi_len(v: u32) -> u8 {
	let mut v = v;
	let mut l = 0;

	loop {
		v = v >> 7;
		l += 1;

		if v == 0 {
			break;
		}
	}

	l
}

pub fn get_properties_len(props: &Option<Vec<MqttProperty>>) -> u32 {
	match props {
		None => 0,
		Some(v) => {
			let mut len = 0;

			for p in v {
				len += 1;

				len += match p {
					MqttProperty::PayloadFormatIndicator(_) |
					MqttProperty::RequestResponseInformation(_) |
					MqttProperty::SubscriptionIdentifierAvailable(_) |
					MqttProperty::RequestProblemInformation(_) |
					MqttProperty::MaximumQoS(_) |
					MqttProperty::RetainAvailable(_) |
					MqttProperty::WildcardSubscriptionAvailable(_) |
					MqttProperty::SharedSubscriptionAvailable(_) => 1,
					MqttProperty::ServerKeepAlive(_) |
					MqttProperty::WillDelayInterval(_) |
					MqttProperty::ReceiveMaximum(_) |
					MqttProperty::TopicAliasMaximum(_) |
					MqttProperty::TopicAlias(_) => 2,
					MqttProperty::MessageExpiryInterval(_) |
					MqttProperty::SessionExpiryInterval(_) |
					MqttProperty::MaximumPacketSize(_) => 4,
					MqttProperty::ContentType(s) |
					MqttProperty::ResponseTopic(s) |
					MqttProperty::AssignedClientIdentifier(s) |
					MqttProperty::AuthenticationMethod(s) |
					MqttProperty::ResponseInformation(s) |
					MqttProperty::ServerReference(s) |
					MqttProperty::ReasonString(s) => 2 + s.as_bytes().len(),
					MqttProperty::UserProperty(k, v) => 4 + k.as_bytes().len() + v.as_bytes().len(),
					MqttProperty::CorrelationData(v) |
					MqttProperty::AuthenticationData(v) => v.len(),
					MqttProperty::SubscriptionIdentifier(v) => vbi_len(*v) as usize
				};
			}

			len as u32
		}
	}
}

pub fn push_properties(buf: &mut Vec<u8>, props: &Option<Vec<MqttProperty>>) {
	push_vbi(buf, get_properties_len(props) as u32);

	match props {
		None => {},
		Some(v) => {
			for p in v {
				push_u8(buf, p.get_prop_type());

				match p {
					MqttProperty::PayloadFormatIndicator(v) |
					MqttProperty::RequestResponseInformation(v) |
					MqttProperty::SubscriptionIdentifierAvailable(v) |
					MqttProperty::RequestProblemInformation(v) |
					MqttProperty::MaximumQoS(v) |
					MqttProperty::RetainAvailable(v) |
					MqttProperty::WildcardSubscriptionAvailable(v) |
					MqttProperty::SharedSubscriptionAvailable(v) => push_u8(buf, *v),
					MqttProperty::ServerKeepAlive(v) |
					MqttProperty::WillDelayInterval(v) |
					MqttProperty::ReceiveMaximum(v) |
					MqttProperty::TopicAliasMaximum(v) |
					MqttProperty::TopicAlias(v) => push_u16(buf, *v),
					MqttProperty::MessageExpiryInterval(v) |
					MqttProperty::SessionExpiryInterval(v) |
					MqttProperty::MaximumPacketSize(v) => push_u32(buf, *v),
					MqttProperty::ContentType(s) |
					MqttProperty::ResponseTopic(s) |
					MqttProperty::AssignedClientIdentifier(s) |
					MqttProperty::AuthenticationMethod(s) |
					MqttProperty::ResponseInformation(s) |
					MqttProperty::ServerReference(s) |
					MqttProperty::ReasonString(s) => {
						push_str(buf, s);
					},
					MqttProperty::UserProperty(k, v) => {
						push_str(buf, k);
						push_str(buf, v);
					},
					MqttProperty::CorrelationData(v) |
					MqttProperty::AuthenticationData(v) => push_binary_data(buf, v),
					MqttProperty::SubscriptionIdentifier(v) => push_vbi(buf, *v)
				}
			}
		}
	}
}

fn create_packet(pt: u8, f: u8) -> MqttPacket {
	match pt {
		1 => MqttPacket::Connect(ConnectProps {
			protocol_name: "".to_string(),
			protocol_level: 0,
			will_retain: false,
			will_qos: 0,
			clean_session: false,
			keep_alive: 0,
			client_identifier: "".to_string(),
			will_topic: None,
			will_message: None,
			user_name: None,
			password: None,
			properties: None,
			will_properties: None
		}),
		2 => MqttPacket::Connack(ConnackProps{
			session_present: false,
			return_code: ConnackReturnCode::Accepted,
			properties: None
		}),
		3 => MqttPacket::Publish(PublishProps{
			dup: (f & 8) == 8,
			qos: (f >> 1) & 4,
			retain: (f & 1) == 1,
			topic_name: String::new(),
			packet_identifier: 0,
			payload: Vec::new(),
			properties: None
		}),
		4 => MqttPacket::Puback(PubackProps {
			packet_identifier: 0,
			reason_code: 0,
			properties: None
		}),
		5 => MqttPacket::Pubrec(PubrecProps {
			packet_identifier: 0,
			reason_code: 0,
			properties: None
		}),
		6 => MqttPacket::Pubrel(PubrelProps {
			packet_identifier: 0,
			reason_code: 0,
			properties: None
		}),
		7 => MqttPacket::Pubcomp(PubcompProps {
			packet_identifier: 0,
			reason_code: 0,
			properties: None
		}),
		8 => MqttPacket::Subscribe(SubscribeProps {
			packet_identifier: 0,
			topics: Vec::new(),
			properties: None
		}),
		9 => MqttPacket::Suback(SubackProps{
			packet_identifier: 0,
			topics: Vec::new(),
			properties: None
		}),
		10 => MqttPacket::Unsubscribe(UnsubscribeProps {
			packet_identifier: 0,
			topics: Vec::new(),
			properties: None
		}),
		11 => MqttPacket::Unsuback(UnsubackProps {
			packet_identifier: 0,
			reason_code: 0,
			properties: None
		}),
		12 => MqttPacket::Pingreq,
		13 => MqttPacket::Pingresp,
		14 => MqttPacket::Disconnect(DisconnectProps {
			reason_code: 0,
			properties: None
		}),
		_ => MqttPacket::Reserved
	}
}

pub fn read_vbi(buf: &[u8], pos: &mut usize) -> Result<u32, Error> {
	let mut vbi: u32 = 0;
	let mut multiplier: u8 = 0;

	loop {
		if buf.len() <= *pos {
			return Err(Error::PacketTooShort);
		}

		let b = buf[*pos];

		*pos += 1;

		let encoded_byte = (b & 0x7f) as u32;
		vbi += encoded_byte << multiplier;
		multiplier += 7;

		if multiplier > 21 {
			return Err(Error::MalformedRemainingLength);
		}

		if b & 0x80 == 0 {
			break;
		}
	}

	Ok(vbi)
}

pub fn read_properties(buf: &[u8], pos: &mut usize) -> Result<Option<Vec<MqttProperty>>, Error> {
	let props_len = read_vbi(buf, pos)? as usize;
	let mut ret = None;

	if props_len > 0 {
		//props subbuffer
		let pbuf = &buf[*pos..][..props_len];
		let mut ppos = 0;
		let mut props = Vec::new();

		while ppos < pbuf.len() {
			let prop_type = read_u8(pbuf, &mut ppos)?;

			let prop = match prop_type {
				1 => MqttProperty::PayloadFormatIndicator(read_u8(pbuf, &mut ppos)?),
				2 => MqttProperty::MessageExpiryInterval(read_u32(pbuf, &mut ppos)?),
				3 => MqttProperty::ContentType(read_str_r(pbuf, &mut ppos)?),
				8 => MqttProperty::ResponseTopic(read_str_r(pbuf, &mut ppos)?),
				9 => MqttProperty::CorrelationData(read_binary_data(pbuf, &mut ppos)?),
				11 => MqttProperty::SubscriptionIdentifier(read_vbi(pbuf, &mut ppos)?),
				17 => MqttProperty::SessionExpiryInterval(read_u32(pbuf, &mut ppos)?),
				18 => MqttProperty::AssignedClientIdentifier(read_str_r(pbuf, &mut ppos)?),
				19 => MqttProperty::ServerKeepAlive(read_u16(pbuf, &mut ppos)?),
				21 => MqttProperty::AuthenticationMethod(read_str_r(pbuf, &mut ppos)?),
				22 => MqttProperty::AuthenticationData(read_binary_data(pbuf, &mut ppos)?),
				23 => MqttProperty::RequestProblemInformation(read_u8(pbuf, &mut ppos)?),
				24 => MqttProperty::WillDelayInterval(read_u16(pbuf, &mut ppos)?),
				25 => MqttProperty::RequestResponseInformation(read_u8(pbuf, &mut ppos)?),
				26 => MqttProperty::ResponseInformation(read_str_r(pbuf, &mut ppos)?),
				28 => MqttProperty::ServerReference(read_str_r(pbuf, &mut ppos)?),
				31 => MqttProperty::ReasonString(read_str_r(pbuf, &mut ppos)?),
				33 => MqttProperty::ReceiveMaximum(read_u16(pbuf, &mut ppos)?),
				34 => MqttProperty::TopicAliasMaximum(read_u16(pbuf, &mut ppos)?),
				35 => MqttProperty::TopicAlias(read_u16(pbuf, &mut ppos)?),
				36 => MqttProperty::MaximumQoS(read_u8(pbuf, &mut ppos)?),
				37 => MqttProperty::RetainAvailable(read_u8(pbuf, &mut ppos)?),
				38 => MqttProperty::UserProperty(read_str_r(pbuf, &mut ppos)?, read_str_r(pbuf, &mut ppos)?),
				39 => MqttProperty::MaximumPacketSize(read_u32(pbuf, &mut ppos)?),
				40 => MqttProperty::WildcardSubscriptionAvailable(read_u8(pbuf, &mut ppos)?),
				41 => MqttProperty::SubscriptionIdentifierAvailable(read_u8(pbuf, &mut ppos)?),
				42 => MqttProperty::SharedSubscriptionAvailable(read_u8(pbuf, &mut ppos)?),
				x => return Err(Error::BadPropertyNum(x))
			};

			props.push(prop);
		}

		*pos += props_len;

		ret = Some(props);
	}

	Ok(ret)
}

pub fn get_packet_type(buf: &[u8]) -> u8 {
	(buf[0] & 0xf0) >> 4
}
