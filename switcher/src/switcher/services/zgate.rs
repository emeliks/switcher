use serde::{ Serialize, Deserialize };
use crate::{ switcher::{ SwMessage, System, sw_device::{ self } } };
use std::{ mem::take, time::{ Instant, Duration }, pin::Pin, task::{ Context, Poll, Waker}, sync::{ Arc, Mutex }, collections::HashMap };
use crate::{ compat::{ sync::Mutex as AsyncMutex, task, channel::{ self, unbounded, Sender } }, switcher::{ SwConn, SwZigbeeEvent, SwData, SwDataValue, SwZbDataValue, SwDataSource, SwServiceType, SwDataEventType, SwDeviceInstance, SwServerData, SwValue, SwZigbeeExtensionType } };
use zigbee::{ ezsp::{ self, AshProcessor }, zcl::{ self, ZclFrame, AttributeRecord, AttributeValue }, zdp::{ self, ZdoCommand, ZdpFrame }, tuya::{ TuyaCommand } };
use core::future::Future;
use frames::{ self, FrameRead, FrameWrite };
use rand::Rng;

//generic types for zigbee messages and commands

#[derive(Debug)]
pub enum ZigbeeData {
	IncomingMessage {
		profile_id: u16,
		cluster_id: u16,
		source_endpoint: u8,
		sender: u16,
		sender_eui64: Option<u64>,
		message_contents: Vec<u8>,
	},
	IncomingNode {
		node_id: u16,
		node_eui64: u64,
		is_child_join: bool
	},
	OutgoingMessage {
		profile_id: u16,
		cluster_id: u16,
		endpoint: u8,
		receiver: u16,
		message_contents: Vec<u8>,
	},
	LocalEui64 {
		eui64: u64
	}
}

#[derive(Debug)]
pub enum Error {
	Ezsp(ezsp::Error),
	Frames(frames::Error),
	SerialPort(serialport::Error),
	RuntimeError(String),
	FutureTimeout,
	RecvError(channel::RecvError),
	SendError(channel::SendError<SwMessage>),
	Parse(std::num::ParseIntError),
	Zcl(zigbee::zcl::Error),
	Zdp(zigbee::zdp::Error),
	SerdeJson(serde_json::Error),
}

impl core::fmt::Display for Error {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		match self {
			Self::Ezsp(e) => e.fmt(f),
			Self::Frames(e) => e.fmt(f),
			Self::SerialPort(e) => e.fmt(f),
			Self::RuntimeError(s) => write!(f, "Runtime error: {}", s),
			Self::FutureTimeout => write!(f, "Future timeout error"),
			Self::RecvError(e) => e.fmt(f),
			Self::SendError(e) => e.fmt(f),
			Self::Parse(e) => e.fmt(f),
			Self::Zcl(e) => e.fmt(f),
			Self::Zdp(e) => e.fmt(f),
			Self::SerdeJson(e) => e.fmt(f),
		}
	}
}

impl From<serde_json::Error> for Error {
	fn from(r: serde_json::Error) -> Self {
		Error::SerdeJson(r)
	}
}

impl From<zigbee::zdp::Error> for Error {
	fn from(e: zigbee::zdp::Error) -> Self {
		Error::Zdp(e)
	}
}
impl From<zigbee::zcl::Error> for Error {
	fn from(e: zigbee::zcl::Error) -> Self {
		Error::Zcl(e)
	}
}

impl From<std::num::ParseIntError> for Error {
	fn from(r: std::num::ParseIntError) -> Self {
		Error::Parse(r)
	}
}

impl From<channel::RecvError> for Error {
	fn from(r: channel::RecvError) -> Self {
		Error::RecvError(r)
	}
}

impl From<channel::SendError<SwMessage>> for Error {
	fn from(r: channel::SendError<SwMessage>) -> Self {
		Error::SendError(r)
	}
}

impl From<serialport::Error> for Error {
	fn from(r: serialport::Error) -> Self {
		Error::SerialPort(r)
	}
}

impl From<ezsp::Error> for Error {
	fn from(e: ezsp::Error) -> Self {
		Error::Ezsp(e)
	}
}

impl From<frames::Error> for Error {
	fn from(e: frames::Error) -> Self {
		Error::Frames(e)
	}
}

#[derive(Deserialize, Debug)]
pub struct SwZGate {
	id: String,
	provider: SwZGateProvider,
	socket: String,
	network: Option<SwZGateNetwork>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum SwZGateProvider {
	#[serde(rename = "ezsp")]
	Ezsp(EzspProvider)
}

#[derive(Debug, Deserialize, Clone)]
pub struct EzspProvider {
	#[serde(skip_deserializing)]
	sender: Option<Sender<Arc<Mutex<EzspFutureState>>>>,
}

#[derive(Debug)]
struct EzspFutureState {
	sequence: Option<u8>,
	command: Option<ezsp::CommandData>,
	response: Option<Result<ezsp::ResponseData, Error>>,
	waker: Option<Waker>,
	timeout: Option<u16>,	//in loop sleep durations,
	start: Instant
}

pub struct EzspFuture {
	state: Arc<Mutex<EzspFutureState>>,
	valid: bool
}

impl Future for EzspFuture {
	type Output = Result<ezsp::ResponseData, Error>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		if self.valid {
			let mut state = self.state.lock().unwrap();

			if let Some(response) = state.response.take() {
				return Poll::Ready(response);
			}
			else {
				state.waker = Some(cx.waker().clone());
				return Poll::Pending;
			}
		}

		//in case of lost connection
		return Poll::Ready(Err(Error::RuntimeError("Invalid future".to_string())));
	}
}

const ZIGBEE_NODES: &str = "zigbee_nodes";
const NETWORK: &str = "network";
const NCP_TIMEOUT: u16 = 200;
const GATE_CLUSTER: u16 = 0xffff;
const JOIN_DURATION: u8 = 100;

impl EzspProvider {
	pub async fn init<S: FrameRead + FrameWrite + Send + Sync + 'static>(&mut self, s: S, frame_t: async_std::channel::Sender<ZigbeeData>, frame_f: async_std::channel::Receiver<ZigbeeData>, config: &SwZGateConfig) -> Result<(), Error> {

		let (tx_s, rx_e) = unbounded::<Arc<Mutex<EzspFutureState>>>();

		let frame_tc = frame_t.clone();

		self.sender = Some(tx_s);

		let debug = config.debug;

		task::spawn(async move {

			let mut processor = AshProcessor::new(s);
			let mut futures: Vec<Arc<Mutex<EzspFutureState>>> = Vec::new();
			let mut last_sender_eui64 = None;
			let mut sequence: u8 = 0;

			loop {
				for (i, s) in futures.iter().enumerate() {
					let mut s = s.lock().unwrap();

					if let Some(ref t) = s.timeout {
						if s.start.elapsed().as_millis() > *t as u128 {
							s.response = Some(Err(Error::FutureTimeout));

							if let Some(waker) = s.waker.take() {
								waker.wake();
							}

							drop(s);
							futures.swap_remove(i);
							break;
						}
					}
				}

				//try - not to block loop
				if let Ok(fs) = rx_e.try_recv() {
					futures.push(fs);
				}

				{
					let mut data = None;
					//search for waiting future

					for s in futures.iter_mut() {
						let mut s = s.lock().unwrap();
						if s.sequence == None {
							s.sequence = Some(processor.sequence);
							data = take(&mut s.command);

							break;
						}
					}

					if let None = data {
						if let Ok(ft) = frame_f.try_recv() {
							//incoming commands to processor

							if debug > 0 {
								println!("incoming message to provider: {:?}", ft);
							}

							match ft {
								ZigbeeData::OutgoingMessage{
									cluster_id: GATE_CLUSTER,
									message_contents,
									..
								} => {
									//internal commands

									//a liitle bit redundant...
									let zf = ZclFrame::from_buf(message_contents.as_slice(), GATE_CLUSTER, zigbee::zcl::CommandExtensionType::Raw);

									if let Ok(ZclFrame { command: zcl::Command::Generic(zcl::GenericCommand::WriteAttributes { values }), .. }) = zf {
										for value in values {
											if let AttributeRecord{identifier: 0, data: AttributeValue::Bool{ val: b }} = value {
												data = Some(
													ezsp::CommandData::PermitJoining { duration: if b { JOIN_DURATION } else { 0 } }
												);
											}
										}
									}
								},
								ZigbeeData::OutgoingMessage{
									profile_id,
									cluster_id,
									endpoint,
									receiver,
									message_contents
								} => {
									let c = ezsp::CommandData::SendUnicast {
										r#type: ezsp::EmberOutgoingMessageType::EmberOutgoingDirect,
										index_or_destination: receiver,
										aps_frame: ezsp::EmberApsFrame{
											profile_id,
											cluster_id,
											source_endpoint: endpoint,
											destination_endpoint: endpoint,
											options: ezsp::EMBER_APS_OPTION_ENABLE_ROUTE_DISCOVERY,
											group_id: 0,
											sequence
										},
										message_tag: 1,
										message_length: message_contents.len() as u8,
										message_contents
									};

									sequence += 1;

									if debug > 0 {
										println!("prepared frame for processor: {:?}", c);
									}

									data = Some(c);
								},
								_ => {
								}
							}
						}
					}

					match processor.run(data) {
						Ok(ret) => {
							//wake up waiting future

							if let Some((response_data, sequence)) = ret {
								if debug > 0 {
									println!("frame from processor: {:?}", response_data);
								}

								match response_data {
									ezsp::ResponseData::StackStatusHandler { status } => {

										match status {
											ezsp::EmberStatus::EmberNetworkOpened |
											ezsp::EmberStatus::EmberNetworkClosed => {
												let val = match status {
													ezsp::EmberStatus::EmberNetworkOpened => true,
													_ => false
												};

												//gate control - own cluster

												if let Ok(message_contents) = ZclFrame::from_command(zcl::Command::Generic(zcl::GenericCommand::ReportAttributes{values: vec![zcl::AttributeRecord{identifier: 0, data: AttributeValue::Bool{val}}]})).to_bytes() {
													let _ = frame_t.send(ZigbeeData::IncomingMessage {
														profile_id: 260,
														cluster_id: GATE_CLUSTER,
														source_endpoint: 0,
														sender: 0,
														sender_eui64: Some(0),
														message_contents
													}).await;
												}
											},
											_ => {}
										}
									},
									ezsp::ResponseData::IncomingSenderEui64Handler { sender_eui64 } => {
										last_sender_eui64 = Some(sender_eui64);
									},
									ezsp::ResponseData::ChildJoinHandler { child_id: node_id, child_eui64: node_eui64, .. } |
									ezsp::ResponseData::TrustCenterJoinHandler{ new_node_id: node_id, new_node_eui64: node_eui64, .. } => {
										let _ = frame_t.send(ZigbeeData::IncomingNode {
											node_id: node_id,
											node_eui64,
											is_child_join: match response_data {
												ezsp::ResponseData::ChildJoinHandler { .. } => true,
												//some devices do not send child join, so joining matching by this
												ezsp::ResponseData::TrustCenterJoinHandler { policy_decision: ezsp::EmberJoinDecision::EmberUsePreconfiguredKey, .. } => true,
												ezsp::ResponseData::TrustCenterJoinHandler { policy_decision: ezsp::EmberJoinDecision::EmberSendKeyInTheClear, .. } => true,
												_ => false
											}
										}).await;
									},
									ezsp::ResponseData::IncomingMessageHandler{ message_contents, aps_frame, sender, .. } => {
										let _ = frame_t.send(ZigbeeData::IncomingMessage {
											profile_id: aps_frame.profile_id,
											cluster_id: aps_frame.cluster_id,
											source_endpoint: aps_frame.source_endpoint,
											sender,
											sender_eui64: last_sender_eui64,
											message_contents
										}).await;

										last_sender_eui64 = None;
									},
									_ => {
										for (i, s) in futures.iter().enumerate() {
											let mut s = s.lock().unwrap();

											if s.sequence == Some(sequence) {
												//wake
												s.response = Some(Ok(response_data));

												if let Some(waker) = s.waker.take() {
													waker.wake();
												}

												drop(s);
												futures.swap_remove(i);
												break;
											}
										}
									}
								}
							}
						},
						Err(e) => {
							//inform awaiting futures about error

							println!("Processor error: {}", e);

							rx_e.close();

							for s in futures {
								let mut s = s.lock().unwrap();

								if s.sequence.is_some() {
									//wake
									s.response = Some(Err(Error::RuntimeError(e.to_string())));

									if let Some(waker) = s.waker.take() {
										waker.wake();
									}
								}
							}

							break;
						}
					}
				}

				//need sleep
				task::sleep(Duration::from_millis(1)).await;
			}
		});

		//ncp needs wait after reset, else returns error 6
		task::sleep(Duration::from_millis(2000)).await;

		//default config - based on zgate nspanel
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigSecurityLevel, 0x0005).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigAddressTableSize, 0x0002).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigTrustCenterAddressCacheSize, 0x0002).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigStackProfile, 0x0002).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigIndirectTransmissionTimeout, 0x1E00).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigMaxHops, 0x001E).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigTxPowerMode, 0x8000).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigSupportedNetworks, 0x0001).await?;
		self.ezsp_set_value_u32(ezsp::EzspValueId::EzspValueEndDeviceKeepAliveSupportMode, 0x00000003).await?;
		self.ezsp_set_policy(ezsp::EzspPolicyId::EzspBindingModificationPolicy, ezsp::EzspDecisionId::EzspCheckBindingModificationsAreValidEndpointClusters).await?;
		self.ezsp_set_policy(ezsp::EzspPolicyId::EzspMessageContentsInCallbackPolicy, ezsp::EzspDecisionId::EzspMessageTagAndContentsInCallback).await?;
		self.ezsp_set_value_u32(ezsp::EzspValueId::EzspValueMaximumIncomingTransferSize, 0x00000052).await?;
		self.ezsp_set_value_u32(ezsp::EzspValueId::EzspValueMaximumOutgoingTransferSize, 0x00000052).await?;
		self.ezsp_set_value_u16(ezsp::EzspValueId::EzspValueCcaThreshold, 0xf2b5).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigBindingTableSize, 0x0010).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigKeyTableSize, 0x0004).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigMaxEndDeviceChildren, 0x0020).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigApsUnicastMessageCount, 0x000A).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigBroadcastTableSize, 0x000F).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigNeighborTableSize, 0x0010).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigPacketBufferCount, 253).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigEndDevicePollTimeout, 0x0008).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigZllGroupAddresses, 0x0000).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigZllRssiThreshold, 0xFFD8).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigTransientKeyTimeoutS, 0x00B4).await?;

		//other config parameters
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigMulticastTableSize, 16).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigSourceRouteTableSize, 16).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigPanIdConflictReportThreshold, 2).await?;
		self.ezsp_set_configuration_value(ezsp::EzspConfigId::EzspConfigApplicationZdoFlags, (ezsp::EMBER_APP_RECEIVES_SUPPORTED_ZDO_REQUESTS | ezsp::EMBER_APP_HANDLES_UNSUPPORTED_ZDO_REQUESTS) as u16).await?;
		self.ezsp_set_value_u16(ezsp::EzspValueId::EzspValueExtendedSecurityBitmask, ezsp::EMBER_JOINER_GLOBAL_LINK_KEY | ezsp::EMBER_NWK_LEAVE_REQUEST_NOT_ALLOWED).await?;

		//based on tasmota
		self.ezsp_set_policy(ezsp::EzspPolicyId::EzspTrustCenterPolicy, ezsp::EzspDecisionId::EzspAllowPreconfiguredKeyJoins).await?;
		self.ezsp_set_policy(ezsp::EzspPolicyId::EzspUnicastRepliesPolicy, ezsp::EzspDecisionId::EzspHostWillNotSupplyReply).await?;
		self.ezsp_set_policy(ezsp::EzspPolicyId::EzspPollHandlerPolicy, ezsp::EzspDecisionId::EzspPollHandlerIgnore).await?;
		self.ezsp_set_policy(ezsp::EzspPolicyId::EzspTcKeyRequestPolicy, ezsp::EzspDecisionId::EzspAllowTcKeyRequestsAndSendCurrentKey).await?;
		self.ezsp_set_policy(ezsp::EzspPolicyId::EzspAppKeyRequestPolicy, ezsp::EzspDecisionId::EzspDenyAppKeyRequests).await?;

		//based on zsmartsystems
		self.ezsp_set_policy(ezsp::EzspPolicyId::EzspTcRejoinsUsingWellKnownKeyPolicy, ezsp::EzspDecisionId::EzspAllowJoins).await?;

		//https://github.com/arendst/Tasmota/discussions/10418 - reportable change

		//add endpoints
		self.ezsp_command(ezsp::CommandData::AddEndpoint {
			endpoint: 1,
			profile_id: ezsp::Z_PROF_HA,
			device_id: 5,
			app_flags: 0,
			input_cluster_count: 19,
			output_cluster_count: 19,
			input_cluster_list: vec![0x0000, 0x0002, 0x0004, 0x0005, 0x0006, 0x0007, 0x0008, 0x000A, 0x0013, 0x0020, 0x0102, 0x0300, 0x0400, 0x0402, 0x0403, 0x0405, 0x0406, 0x0500, 0x0501],
			output_cluster_list: vec![0x0000, 0x0002, 0x0004, 0x0005, 0x0006, 0x0007, 0x0008, 0x000A, 0x0013, 0x0020, 0x0102, 0x0300, 0x0400, 0x0402, 0x0403, 0x0405, 0x0406, 0x0500, 0x0501]
		}, Some(NCP_TIMEOUT)).await?;

		self.ezsp_command(ezsp::CommandData::AddEndpoint {
			endpoint: 0x0b,
			profile_id: ezsp::Z_PROF_HA,
			device_id: 5,
			app_flags: 0,
			input_cluster_count: 0,
			output_cluster_count: 0,
			input_cluster_list: vec![],
			output_cluster_list: vec![]
		}, Some(NCP_TIMEOUT)).await?;

		//values from config
		let pan_id = config.network.pan_id;
		let extended_pan_id = config.network.extended_pan_id;
		let key = config.network.key;

		let radio_channel = config.network.channel;
		let radio_tx_power = 9;

		self.ezsp_command(ezsp::CommandData::SetInitialSecurityState {
			state: ezsp::EmberInitialSecurityState {
				bitmask: ezsp::EMBER_TRUST_CENTER_GLOBAL_LINK_KEY | ezsp::EMBER_HAVE_PRECONFIGURED_KEY | ezsp::EMBER_HAVE_NETWORK_KEY | ezsp::EMBER_NO_FRAME_COUNTER_RESET,
				preconfigured_key: ezsp::EmberKeyData { contents: [0x5A, 0x69, 0x67, 0x42, 0x65, 0x65, 0x41, 0x6C, 0x6C, 0x69, 0x61, 0x6E, 0x63, 0x65, 0x30, 0x39] },	// well known key "ZigBeeAlliance09"
				network_key: ezsp::EmberKeyData { contents: key },
				network_key_sequence_number: 0,
				preconfigured_trust_center_eui64: 0
			}
		}, Some(NCP_TIMEOUT)).await?;

		//wait for status: EmberNetworkUp
		//todo!

		//network form
		self.ezsp_command(ezsp::CommandData::FormNetwork(ezsp::EmberNetworkParameters {
			extended_pan_id,
			pan_id,
			radio_tx_power,
			radio_channel,
			join_method: ezsp::EmberJoinMethod::EmberUseMacAssociation,
			nwk_manager_id: 0xffff,
			nwk_update_id: 0,
			channels: 0
		}), Some(NCP_TIMEOUT)).await?;

		//wait for network open
		task::sleep(Duration::from_millis(200)).await;

		// auto subscribe to group 0 in slot 0
		self.ezsp_command(ezsp::CommandData::SetMulticastTableEntry{ index: 0, value: ezsp::EmberMulticastTableEntry{ multicast_id: 0, endpoint: 1, network_index: 0 }}, Some(NCP_TIMEOUT)).await?;

		let mut index = 1;

		//i.e. ikea tradfri remote controller

		for multicast_id in &config.multicast_groups {
			self.ezsp_command(ezsp::CommandData::SetMulticastTableEntry{ index, value: ezsp::EmberMulticastTableEntry{ multicast_id: *multicast_id, endpoint: 1, network_index: 0 }}, Some(NCP_TIMEOUT)).await?;

			index += 1;
		}

		//local eui64
		let x = self.ezsp_command(ezsp::CommandData::GetEui64, Some(NCP_TIMEOUT)).await?;

		if let ezsp::ResponseData::GetEui64{ eui64 } = x {
			let _ = frame_tc.send(ZigbeeData::LocalEui64{ eui64 }).await;
		}

		self.ezsp_command(ezsp::CommandData::PermitJoining { duration: JOIN_DURATION }, Some(NCP_TIMEOUT)).await?;

		Ok(())
	}

	async fn ezsp_set_configuration_value(&mut self, config_id: ezsp::EzspConfigId, value: u16) -> Result<ezsp::ResponseData, Error> {
		let ret = self.ezsp_command(ezsp::CommandData::SetConfigurationValue { config_id: config_id, value }, Some(NCP_TIMEOUT)).await?;

		Ok(ret)
	}

	async fn ezsp_set_value_u16(&mut self, value_id: ezsp::EzspValueId, value: u16) -> Result<ezsp::ResponseData, Error> {
		let ret = self.ezsp_command(ezsp::CommandData::SetValue {
			value_id,
			value_length: 2,
			value: value.to_le_bytes().to_vec()
		}, Some(NCP_TIMEOUT)).await?;

		Ok(ret)
	}

	async fn ezsp_set_value_u32(&mut self, value_id: ezsp::EzspValueId, value: u32) -> Result<ezsp::ResponseData, Error> {
		let ret = self.ezsp_command(ezsp::CommandData::SetValue {
			value_id,
			value_length: 4,
			value: value.to_le_bytes().to_vec()
		}, Some(NCP_TIMEOUT)).await?;

		Ok(ret)
	}

	async fn ezsp_set_policy(&mut self, policy_id: ezsp::EzspPolicyId, decision_id: ezsp::EzspDecisionId) -> Result<ezsp::ResponseData, Error> {
		let ret = self.ezsp_command(ezsp::CommandData::SetPolicy { policy_id, decision_id }, Some(NCP_TIMEOUT)).await?;

		Ok(ret)
	}

	pub fn ezsp_command(&mut self, data: ezsp::CommandData, timeout: Option<u16>) -> EzspFuture {
		let state = Arc::new(Mutex::new(EzspFutureState { command: Some(data), response: None, waker: None, sequence: None, timeout, start: Instant::now() }));
		let mut valid = false;

		if let Some(sender) = &self.sender {
			let r = sender.try_send(state.clone());
			valid = r.is_ok();

			if !valid {
				println!("Error sending state: {}", r.err().unwrap());
			}
		}

		EzspFuture {
			state,
			valid
		}
	}
}

impl SwZGateProvider {
	async fn init<S: FrameRead + FrameWrite + Send + Sync + 'static>(&mut self, s: S, frame_t: async_std::channel::Sender<ZigbeeData>, frame_f: async_std::channel::Receiver<ZigbeeData>, config: &SwZGateConfig) -> Result<(), Error> {
		match self {
			Self::Ezsp(e) => Ok(e.init(s, frame_t, frame_f, config).await?)
		}
	}

/*
	async fn ping(&mut self) -> Result<(), Error> {
		match self {
			Self::Ezsp(e) => {
				let _resp = e.ezsp_command(ezsp::CommandData::Nop, Some(NCP_TIMEOUT)).await?;
			}
		}

		Ok(())
	}
*/
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SwZGateNetwork {
	pan_id: u16,
	#[serde(with = "hex::serde")]
	extended_pan_id: [u8; 8],
	#[serde(with = "hex::serde")]
	key: [u8; 16],
	channel: u8
}

pub struct SwZGateConfig {
	debug: u8,
	multicast_groups: Vec<u16>,
	network: SwZGateNetwork
}

impl SwZGate {
	pub async fn run(&self, system: &mut System) -> Result<(), sw_device::Error> {

		let socket = self.socket.clone();
		let mut provider = self.provider.clone();
		let mut eui_map: HashMap<u16, u64> = HashMap::new();
		let mut rev_eui_map: HashMap<u64, u16> = HashMap::new();
		let devices_by_id: HashMap<u64, (usize, Arc<SwDeviceInstance>)> = HashMap::new();
		let devices_by_id_am = Arc::new(AsyncMutex::new(devices_by_id));
		let devices_by_id_amr = devices_by_id_am.clone();
		let m_id = self.id.clone();
		let local_eui64 = Arc::new(AsyncMutex::new(0));
		let local_eui64_c = local_eui64.clone();

		//primary channel
		let (sender_id, rx, tx) = system.get_channel();

		let network = if let Some(cur_network) = &self.network {
			cur_network.clone()
		}
		else {
			if let Some(o) = system.run_config.get(NETWORK) {
				serde_json::from_value(o.clone())?
			}
			else {
				//generate new network parameters

				let mut rng = rand::thread_rng();

				SwZGateNetwork {
					pan_id: rng.gen(),
					extended_pan_id: rng.gen(),
					key: rng.gen(),
					channel: 20
				}
			}
		};

		//store in run config file
		_ = tx.send(SwMessage::RunConfig{ data: serde_json::json!({NETWORK: network})}).await;

		let mut config = SwZGateConfig{
			debug: system.config.debug.unwrap_or(0),
			multicast_groups: Vec::new(),
			network
		};

		for dev in &system.config.device_instances {
			if let Some(consts) = &dev.consts {
				if let Some(SwValue::Numeric(multicast_group_id)) = consts.get("multicast_group_id") {
					config.multicast_groups.push(*multicast_group_id as u16);
				}
			}
		}

		//try read rev_eui from run config
		if let Some(serde_json::Value::Object(o)) = system.run_config.get(ZIGBEE_NODES) {
			if config.debug > 0 {
				println!("run config zigbee nodes: {:?}", o);
			}

			for (k, v) in o {
				if let (Ok(eui64), serde_json::Value::Number(n)) = (u64::from_str_radix(k, 16), v) {
					if let Some(n) = n.as_u64() {
						rev_eui_map.insert(eui64, n as u16);
						eui_map.insert(n as u16, eui64);
					}
				}
			}
		}

		let rev_eui_map_am = Arc::new(AsyncMutex::new(rev_eui_map));
		let rev_eui_map_amc = rev_eui_map_am.clone();
		let txr = tx.clone();

		tx.send(SwMessage::RegisterServer{ sender_id, server_id: self.id.clone(), server_data: SwServerData::Zigbee }).await?;

		//communication with provider's loop - data to provider
		let (tx_to, rx_to) = unbounded::<ZigbeeData>();

		let mut zcl_transaction_sequence_number: u8 = 0;
		let mut zdp_sequence_number: u8 = 0;

		task::spawn(async move {
			loop {
				if let Err(e) = async {
					let received = rx.recv().await?;

					match received {
						SwMessage::RegisterService{ sender_id: device_id, server_id: _, service_type } => {
							if let SwServiceType::ZigbeeDev{ dev } = service_type {

								//remember device
								if let Some(consts) = &dev.consts {
									if let Some(SwValue::String(zb_id)) = consts.get("zb_id") {
										let u_zb_id = u64::from_str_radix(zb_id, 16)?;
										devices_by_id_am.lock().await.insert(u_zb_id, (device_id, dev));
									}
								}

								//send info to dev - after provider is initialized
							}
						},

						SwMessage::DevRawVal{ value: SwValue::Map(mut map), .. } => {
							if config.debug > 0 {
								println!("Zigbee raw val contents: {:?}", map);
							}

							if let Some(SwValue::String(zb_id)) = map.get("zb_id") {
								let u_zb_id = u64::from_str_radix(zb_id, 16)?;
								let mut cluster_id = if let Some(SwValue::Numeric(cluster_id)) = map.get("cluster_id") { *cluster_id as u16 } else { 0 };
								let profile_id = if let Some(SwValue::Numeric(profile_id)) = map.get("profile_id") { *profile_id as u16 } else { 260 };
								let mut endpoint = if let Some(SwValue::Numeric(endpoint)) = map.get("endpoint") { *endpoint as u8 } else { 1 };

								let mut message_contents = None;

								match profile_id {
									0 => {
										//zdo command

										if let Some(cmd) = map.remove("cmd") {
											let command = match cmd {
												SwValue::Vec(v) => {
													let mut vec = Vec::with_capacity(v.len());

													for val in v {
														if let SwValue::Numeric(n) = val {
															vec.push(n as u8);
														}
													}

													Some(ZdoCommand::from_buf(&vec, cluster_id)?)
												},
												SwValue::String(s) => {
													let jv = serde_json::from_str(&s)?;

													Some(ZdoCommand::from_json(jv)?)
												},
												SwValue::Map(_) => {
													match cmd.into_json() {
														//do not have impl from sw_device::Error
														Ok(jv) => Some(ZdoCommand::from_json(jv)?),
														Err(e) => {
															println!("Error creating zdo command: {}", e);

															None
														}
													}
												},
												_ => None
											};

											if let Some(mut command) = command {
												//helper for set fields
												match command {
													ZdoCommand::BindReq(ref mut br) => {
														br.src_address = u_zb_id;
														br.src_endp = 1;
														br.dst_addr = zdp::BindReqAddr::Long {
															dst_addr: *local_eui64_c.lock().await,
															dst_endp: 1
														};

														endpoint = 0;
													},
													_ => {
													}
												}

												let mut zdp_frame = ZdpFrame::from_command(command);

												zdp_frame.sequence_number = zdp_sequence_number;

												if zdp_frame.command_no != 0 {
													//when command created from json, no need to set cluster id
													cluster_id = zdp_frame.command_no;
												}

												zdp_sequence_number += 1;

												message_contents = Some(zdp_frame.to_bytes()?);

												if config.debug > 0 {
													println!("prepared zdp frame: {:?}, {:0x?}", zdp_frame, message_contents);
												}
											}
										}
									},
									260 => {
										//zcl command packed in zcl frame

										//frame type 1 - cluster specific, 0 - generic
										let mut frame_type = if let Some(SwValue::Numeric(frame_type)) = map.get("frame_type") { *frame_type as u8 } else { 1 };
										let mut command = None;

										let mut command_extension_type = SwZigbeeExtensionType::Raw;

										if let Some((_, dev)) = devices_by_id_am.lock().await.get(&u_zb_id) {
											if let SwDataEventType::Zigbee(SwZigbeeEvent{ extension_type, .. }) = dev.get_data_event() {
												command_extension_type = extension_type.clone();
											}
										}

										let extension_type = match command_extension_type {
											SwZigbeeExtensionType::Raw => zcl::CommandExtensionType::Raw,
											SwZigbeeExtensionType::Tuya => zcl::CommandExtensionType::Tuya
										};

										//cluster specific command
										if let Some(cmd) = map.remove("cmd") {
											command = match cmd {
												//command as single byte
												SwValue::Numeric(n) => Some(zcl::Command::Raw(vec![n as u8])),
												//command as vec of bytes
												SwValue::Vec(v) => {
													let mut vec = Vec::with_capacity(v.len());

													for val in v {
														if let SwValue::Numeric(n) = val {
															vec.push(n as u8);
														}
													}

													Some(zcl::Command::from_buf(&vec, frame_type, cluster_id, extension_type)?)
												},
												SwValue::String(s) => {
													let jv = serde_json::from_str(&s)?;

													Some(zcl::Command::from_json(jv, cluster_id, frame_type)?)
												},
												SwValue::Map(_) => {
													match cmd.into_json() {
														//do not have impl from sw_device::Error
														Ok(jv) => Some(zcl::Command::from_json(jv, cluster_id, frame_type)?),
														Err(e) => {
															println!("Error creating zcl command: {}", e);

															None
														}
													}
												},
												_ => None
											};
										}

										//attribute value

										if let (Some(val), Some(SwValue::Numeric(t)), Some(SwValue::Numeric(i))) = (map.get("val"), map.get("type"), map.get("identifier")) {
											if command_extension_type == SwZigbeeExtensionType::Tuya && cluster_id == 0xef00 {
												//tuya extension attribute

												let sequence_number = 1;

												match val.clone().to_tav(*t as u8) {
													Ok(value) => {
														let data = zigbee::tuya::DpData{
															dp_id: *i as u8,
															value
														};

														command = Some(zcl::Command::Tuya(TuyaCommand::TyDataRequest{ sequence_number, data }));
														frame_type = 1;
													},
													Err(e) => {
														println!("Error converting SwValue {:?} into Tuya value: {}", val, e);
													}
												}
											}
											else {
												//zigbee generic attribute

												let ar = zcl::AttributeRecord{
													identifier: *i as u16,
													data: val.clone().to_av(*t as u8)
												};

												command = Some(zcl::Command::Generic(zcl::GenericCommand::WriteAttributes{ values: vec![ar] }));
												frame_type = 0;
											}
										}

										if let Some(command) = command {
											let frame = ZclFrame{
												control: zcl::ZclFrameControl{
													disable_default_response: config.debug == 0,
													direction: 0, //?0 ?
													manufacturer_specific: 0,
													frame_type
												},
												manufacturer_code: None,
												transaction_sequence_number: zcl_transaction_sequence_number,
												command
											};

											zcl_transaction_sequence_number += 1;

											message_contents = Some(frame.to_bytes()?);	//skipped first byte!

											if config.debug > 0 {
												println!("prepared zcl frame contents: {:?}, {:0x?}", frame, message_contents);
											}
										}
										else {
											println!("DevRawVal for Zigbee requires 'zb_id', 'cluster_id', ('val', 'type' and 'identifier' or 'cmd')");
										}
									},
									_ => {
									}
								}

								if let (Some(message_contents), Some(receiver)) = (message_contents, rev_eui_map_amc.lock().await.get(&u_zb_id)) {
									let m = ZigbeeData::OutgoingMessage{
										profile_id,
										cluster_id,
										endpoint,
										receiver: *receiver,
										message_contents
									};

									_ = tx_to.send(m).await;
								}
								else {
									println!("Short addr of {:#0x} not found!", u_zb_id);
								}
							}
						},
						x => {
							if config.debug > 0 {
								println!("zgate received: {:?}", x);
							}
						}
					}

					Ok::<(), Error>(())
				}.await {
					println!("Error in zgate server loop: {:?}", e);
				}
			}
		});

		task::spawn(async move {
			loop {
				#[allow(unreachable_code)]
				if let Err(e) = async {

				let port = serialport::new(&socket, 115_200).
					data_bits(serialport::DataBits::Eight).
					parity(serialport::Parity::None).
					stop_bits(serialport::StopBits::One).
					timeout(core::time::Duration::from_millis(100)).
					open_native()?;

					//communication with provider's loop - data from provider
					let (tx_f, rx_f) = unbounded::<ZigbeeData>();

					provider.init(port, tx_f, rx_to.clone(),&config).await?;

					_ = tx.send(SwMessage::Initialized { sender_id, msg: format!("Initialized Zigbee provider") } ).await;

					for (u_zb_id, (device_id, dev)) in devices_by_id_amr.lock().await.iter() {
						txr.send(
							SwMessage::SendTo{
								sender_id,
								receiver_id: *device_id,
								msg: Box::new(
									SwMessage::ServiceRegistered{
										sender_id,
										server_id: m_id.clone(),
										server_data: SwServerData::Zigbee
									}
								)
							}
						).await?;

						println!("Register Zigbee device: {}, {}, zb_id: {:0x?}", device_id, dev.id, u_zb_id);
					}

					loop {
						let f = rx_f.recv().await?;

						match f {
							ZigbeeData::LocalEui64{ eui64 } => {
								*local_eui64.lock().await = eui64;
							},
							ZigbeeData::IncomingNode{ node_id, node_eui64, is_child_join } => {

								if config.debug > 0 {
									println!("new node: {}, {:#016x?}", node_id, node_eui64);
								}

								let new_node = rev_eui_map_am.lock().await.get(&node_eui64) != Some(&node_id);

								if new_node {
									//send data to run config
									_ = tx.send(SwMessage::RunConfig{ data: serde_json::json!({ZIGBEE_NODES: {format!("{:016x}", node_eui64): node_id}})}).await;	//no #..x!
								}

								eui_map.insert(node_id, node_eui64);
								rev_eui_map_am.lock().await.insert(node_eui64, node_id);

								//on connect device
								if is_child_join {
									if let Some((dev_service_id, _)) = devices_by_id_amr.lock().await.get(&node_eui64) {
										tx.send(SwMessage::SendTo{ sender_id, receiver_id: *dev_service_id, msg: Box::new(SwMessage::DevConn{ sender_id, conn: SwConn::Zigbee{ server_sender_id: sender_id } }) }).await?;
									}
								}
							},
							ZigbeeData::IncomingMessage { message_contents, profile_id, cluster_id, sender, sender_eui64, .. } => {
								if let Some(eui64) = sender_eui64 {

									let new_node = rev_eui_map_am.lock().await.get(&eui64) != Some(&sender);

									if new_node {
										//send data to run config
										_ = tx.send(SwMessage::RunConfig{ data: serde_json::json!({ZIGBEE_NODES: {format!("{:016x}", eui64): sender}})}).await;	//no #..x!
									}

									eui_map.insert(sender, eui64);
									rev_eui_map_am.lock().await.insert(eui64, sender);
								}

								if let Some(eui64) = eui_map.get(&sender) {

									if config.debug > 0 {
										println!("message from: {:#016x}", eui64);
									}

									let mut vals = None;
									let mut service_id = None;

									if let Some((dev_service_id, dev)) = devices_by_id_amr.lock().await.get(eui64) {
										service_id = Some(*dev_service_id);

										let command_extension_type = match dev.get_data_event() {
											SwDataEventType::Zigbee(SwZigbeeEvent{extension_type: SwZigbeeExtensionType::Tuya, ..}) => zigbee::zcl::CommandExtensionType::Tuya,
											_ => zigbee::zcl::CommandExtensionType::Raw
										};

										match profile_id {
											0 => {
												let zf = ZdpFrame::from_buf(&message_contents, cluster_id);

												if config.debug > 0 {
													println!("incoming zdp frame: {:?}", zf);
												}

												//todo zdp responses
											},
											260 => {
												let zf = ZclFrame::from_buf(message_contents.as_slice(), cluster_id, command_extension_type);

												if config.debug > 0 {
													println!("zcl frame: {:?}", zf);

													if let Ok(ref f) = zf {
														println!("incoming zcl command as json: {:?}", f.command.to_json());
													}
												}

												match zf {
													//finally we've got zigbee data ready to send to switcher

													//generic zigbee attributes
													Ok(ZclFrame { command: zcl::Command::Generic(zcl::GenericCommand::ReadAttributesResponse { values }), .. }) => {
														let mut v = Vec::with_capacity(values.len());

														for value in values {
															//extracted values

															if let Some(data) = value.data {
																if config.debug > 0 {
																	println!("incoming read atributes response: cluster: {:#0x}, Sender: {:#016x}, atribute_id: {}, value: {:?}", cluster_id, eui64, value.identifier, data);
																}

																v.push(SwData {
																	val: SwDataValue::Zb(SwZbDataValue::Attribute{ identifier: value.identifier, value: data }),
																	src: SwDataSource::Zb { cluster_id, eui64: *eui64 }
																});
															}
														}

														vals = Some(v);
													},
													Ok(ZclFrame { command: zcl::Command::Generic(zcl::GenericCommand::ReportAttributes { values }), .. }) => {
														let mut v = Vec::with_capacity(values.len());

														for value in values {
															//extracted values

															if config.debug > 0 {
																println!("incoming report attributes: cluster: {:#0x}, Sender: {:#016x}, atribute_id: {}, value: {:?}", cluster_id, eui64, value.identifier, value.data);
															}

															v.push(SwData {
																val: SwDataValue::Zb(SwZbDataValue::Attribute{ identifier: value.identifier, value: value.data }),
																src: SwDataSource::Zb { cluster_id, eui64: *eui64 }
															});
														}

														vals = Some(v);
													},
													//tuya attributes
													Ok(ZclFrame{ command: zcl::Command::Tuya(TuyaCommand::TyDataResponse { data, .. }), .. }) |
													Ok(ZclFrame{ command: zcl::Command::Tuya(TuyaCommand::TyDataReport { data, .. }), .. }) => {
														//extracted values
														//encapsutating data point id in attribute id
														let av: AttributeValue = data.value.into();

														if config.debug > 0 {
															println!("incoming tuya attributes: cluster: {:#0x}, Sender: {:#016x}, atribute_id: {}, value: {:?}", cluster_id, eui64, data.dp_id as u16, av);
														}

														vals = Some(vec![SwData {
															val: SwDataValue::Zb(SwZbDataValue::Attribute{ identifier: data.dp_id as u16, value: av}),
															src: SwDataSource::Zb { cluster_id, eui64: *eui64 }
														}]);
													},
													Ok(f) => {
														//zb data as command to detect as json_val or fr_val

														if config.debug > 0 {
															println!("incoming command: cluster: {:#0x}, Sender: {:#016x}, command: {:?}", cluster_id, eui64, f.command);
														}

														vals = Some(vec![SwData {
															val: SwDataValue::Zb(SwZbDataValue::ZclCommand(f.command)),
															src: SwDataSource::Zb { cluster_id, eui64: *eui64 }
														}]);
													},
													_ => {}
												}

												if config.debug > 0 {
													println!("incoming data: {:?}, device: {}", vals, dev.id);
												}
											},
											_ => {
												//error?
											}
										}
									}

									if let (Some(service_id), Some(vals)) = (service_id, vals) {
										if config.debug > 0 {
											println!("Sending message to device handler: {} {:?}", service_id, vals);
										}

										for data in vals {
											//send info to dev
											tx.send(SwMessage::SendTo{
												sender_id,
												receiver_id: service_id,
												msg: Box::new(SwMessage::DevData {
													sender_id,
													data
												})
											}).await?;
										}
									}
								}
								else {
									println!("No eui64 for sender: {}, map: {:?}", sender, eui_map);
								}
							},
							_ => {
							}
						}

						//test connection todo timeout
						
						//provider.ping().await?;
					}

					Ok::<(), Error>(())
				}.await {
					println!("Error in zgate loop: {:?}", e);
				}

				println!("Waiting 5 sec");
				task::sleep(Duration::from_secs(5)).await;
			}
		});

		Ok(())
	}
}