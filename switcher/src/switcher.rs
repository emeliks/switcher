use serde::{ Deserialize, de::{ Deserializer } };
use std::{ collections::{ HashMap, HashSet }, time::Duration, cmp::min, sync::Arc, fs::File };
use crate::compat::{ sync::{ Mutex }, channel::{ unbounded, Sender, Receiver }, task, io::{ timeout }, net::TcpStream };
use core::{ fmt::Debug, ops::{ DerefMut, Deref } };
use chrono::{ self, NaiveTime, TimeZone, prelude::{ DateTime, Local }, Datelike };
use base64::{ Engine as _, engine::general_purpose};
use tokenizer::{ self, Token, Tokenizer, Eval, TokenType, StdEval };
use mqtt::mqtt_packet::{ MqttPacket, PublishProps, MqttProperty };
use net_services::{
	text,
	http::{ HttpHeaders, async_http_connection, HttpConnection },
	compat::{ AsyncWrite },
	tls_client_connection
};
use frames::Frame;
use zigbee::{ zcl::{ AttributeValue, Command } };

use crate::{
	switcher::{ sw_device::{ Error, SwService, SwValue }, services::{ mqtt_client::SwServerMqttClient, mqtt_server::SwServerMqtt, http_server::SwServerHttp, action::SwAction, zgate::SwZGate } },
};

pub mod sw_device;
pub mod services;

#[derive(Deserialize, Debug)]
pub struct Config {
	debug: Option<u8>,
	data_dir: Option<String>,
	ip: String,
	services: Vec<SwService>,
	device_groups: HashMap<String, SwDeviceType>,
	device_instances: Vec<SwDeviceInstance>,
	servers: Vec<SwServer>,
	actions: Vec<SwAction>
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum SwServer {
	#[serde(rename = "mqtt")]
	Mqtt(SwServerMqtt),
	#[serde(rename = "mqtt_client")]
	MqttClient(SwServerMqttClient),
	#[serde(rename = "http")]
	Http(SwServerHttp),
	#[serde(rename = "zgate")]
	Zgate(SwZGate)
}

pub fn get_json_from_path<'a>(mut ret: &'a serde_json::value::Value, path: &'a str) -> Option<&'a serde_json::value::Value> {
	for s in path.split('.') {
		match ret {
			serde_json::value::Value::Object(m) => {
				match m.get(s) {
					Some(r) => { ret = r; },
					None => { return None; }
				}
			},
			serde_json::value::Value::Array(v) => {
				if let Ok(n) = s.parse::<usize>() {
					match v.get(n) {
						Some(r) => { ret = r },
						None => { return None; }
					}
				}
				else {
					return None;
				}
			},
			_ => {
				return None;
			}
		}
	}

	Some(&ret)
}

impl SwServer {
	pub async fn run(&self, system: &mut System) -> Result<(), Error> {
		match self {
			Self::MqttClient(mc) => {
				mc.run(system).await?;
			},
			Self::Mqtt(m) => {
				m.run(system).await?;
			},
			Self::Http(h) => {
				h.run(system).await?;
			},
			Self::Zgate(g) => {
				g.run(system).await?;
			}
		};

		Ok(())
	}
}

#[derive(Deserialize, Default, Debug)]
struct SwDeviceType {
	data_event: SwDataEventType,
	read: Option<HashMap<String, SwEvaluator>>,
	write: Option<HashMap<String, SwEvaluator>>,
	on_run: Option<SwEvaluator>,
	on_connect: Option<SwEvaluator>,
	actions: Option<Vec<SwAction>>
}

#[derive(Deserialize, Debug)]
pub struct SwDeviceInstance {
	id: String,
	group: String,
	data_event: Option<SwDataEventType>,
	consts: Option<HashMap<String, SwValue>>,
	#[serde(skip_deserializing)]
	dev_group: Arc<SwDeviceType>
}

impl SwDeviceInstance {
	pub fn get_data_event(&self) -> &SwDataEventType {
		match &self.data_event {
				Some(data_event) => data_event,
				None => &self.dev_group.data_event
			}
	}

	pub async fn run(self: Arc<Self>, system: &mut System) -> Result<(), Error> {
		let (sender_id, rx, tx) = system.get_channel();
		let txc = tx.clone();
		let vals = Arc::new(Mutex::new(HashMap::new()));
		let dev = self.clone();
		let debug = system.config.debug;

		let data_event = match &self.data_event {
			Some(data_event) => data_event,
			None => &self.dev_group.data_event
		};

		if let Some(on_run) = &self.dev_group.on_run {
			let mut ctx = SwEvalCtx{ dev: Some((dev.clone(), sender_id)), vals_prefix: Some(dev.id.clone()), msgs: Some(Vec::new()), ..Default::default() };

			if let Err(e) = async {
				_ = on_run.eval(&mut ctx)?;

				if let SwEvalCtx{ msgs: Some(msgs), .. } = ctx {
					//send msgs
					for msg in msgs {
						txc.send(msg).await?;
					}
				}
				Ok::<(), Error>(())
			}.await {
				println!("Error run dev {}: {} ", dev.id, e);
			}
		}

		match data_event {
			SwDataEventType::OutSocket(a) => {
				if let Some(ip) = &a.host {
					let port = match a.port {
						Some(p) => p,
						None => match &self.dev_group.data_event {
							SwDataEventType::OutSocket(a) => match a.port {
								Some(p) => p,
								None => 23
							},
							_ => 23
						}
					};

					let addr = format!("{}:{}", ip, port);
					tx.send(SwMessage::Initialized{sender_id, msg: format!("Connecting to addr {}", &addr)}).await?;

					task::spawn(async move {
						loop {
							#[allow(unreachable_code)]
							if let Err(e) = async {
								let mut stream = timeout(Duration::from_millis(2000 as u64), async {
									TcpStream::connect(&addr).await
									}).await?;

								tx.send(SwMessage::Initialized{sender_id, msg: format!("Connected to addr {}", &addr)}).await?;

								//send current connection to write thread
								tx.send(SwMessage::SendTo{ sender_id, receiver_id: sender_id, msg: Box::new(SwMessage::DevConn{ sender_id, conn: SwConn::Tcp{ stream: stream.clone() } }) }).await?;

								loop {
									let s = text::read_line(&mut stream).await?;
									//shortcut to device, no need to select
									tx.send(SwMessage::SendTo{ sender_id, receiver_id: sender_id, msg: Box::new(SwMessage::DevData{ sender_id, data: SwData::from_string(s) }) }).await?;
								}

								Ok::<(), Error>(())
							}.await {
								println!("Error in out socket loop: {:?}", e);
								_ = tx.send(SwMessage::SendTo{ sender_id, receiver_id: sender_id, msg: Box::new(SwMessage::DevConn{ sender_id, conn: SwConn::None }) }).await;
							}

							println!("Waiting 5 sec");
							task::sleep(Duration::from_secs(5)).await;
						}
					});
				}
			},
			SwDataEventType::OutHttp(h) => {
				let read_request = match &h.read_request {
					Some(p) => Some(p),
					None => match &self.dev_group.data_event {
						SwDataEventType::OutHttp(h) => match &h.read_request {
							Some(p) => Some(p),
							None => None
						},
						_ => None
					}
				};

				if let Some(rp) = read_request {
					let interval = if let Some(s) = &h.interval { *s } else { if let SwDataEventType::OutHttp(h) = &self.dev_group.data_event { if let Some(s) = h.interval { s } else { 2000 } } else { 2000 } };
					let rp = rp.clone();
					let vals = vals.clone();

					task::spawn(async move {
						loop {
							if let Err(e) = async {
								//in polled mode we need do fit in desired interval

								let start = Local::now().timestamp_millis();
								let mut ctx = SwEvalCtx{ dev: Some((dev.clone(), sender_id)), ..Default::default() };
								let v_read_request;

								{
									let mut cvals = vals.lock().await;

									//steal vals from mutex
									let mut vs = std::mem::take(cvals.deref_mut());
									ctx.vals = Some(vs);

									v_read_request = rp.eval(&mut ctx)?;
									vs = std::mem::take(&mut ctx.vals).unwrap();

									//give back
									_ = std::mem::replace(cvals.deref_mut(), vs);
								}

								_ = tx.send(SwMessage::SendTo{
									sender_id,
									receiver_id: sender_id,
									msg: Box::new(SwMessage::DevRawVal{
										sender_id,
										value: v_read_request,
										read_reply: true
									})
								}).await;

								let t = Local::now().timestamp_millis() - start;

								if t < interval as i64 {
									task::sleep(Duration::from_millis((interval as i64 - t) as u64)).await;
								}

								Ok::<(), Error>(())
							}.await {
								println!("Error in out http loop: {:?}", e);
							}
						}
					});
				}
			},
			SwDataEventType::InMqtt(m) => {
				tx.send(SwMessage::RegisterService{
					sender_id,
					server_id: m.server_id.clone(),
					service_type: SwServiceType::MqttDev{ dev: self.clone() }
				}).await?;
			},
			SwDataEventType::Zigbee(ze) => {
				tx.send(SwMessage::RegisterService{
					sender_id,
					server_id: ze.server_id.clone(),
					service_type: SwServiceType::ZigbeeDev{ dev: self.clone() }
				}).await?;
			},
			SwDataEventType::TimeInterval(i) => {
				let interval = i.interval as u64;

				if let Some(start_at) = i.start_at {
					let at = Local::now().timezone().from_local_datetime(&Local::now().date_naive().and_time(start_at)).unwrap();
					let now = Local::now();
					let mut d = at - now;

					if d.num_seconds() < 0 {
						if let Some(dd) = chrono::Duration::try_days(1) {
							d = d + dd;
						}
					}

					if let Ok(td) = d.to_std() {
						task::sleep(td).await;
					}
				}

				task::spawn(async move {
					loop {
						if let Err(e) = async {
							tx.send(SwMessage::SendTo{ sender_id, receiver_id: sender_id, msg: Box::new(SwMessage::DevData{ sender_id, data: SwData{ ..Default::default() } }) }).await?;
							task::sleep(Duration::from_millis(interval as u64)).await;

							Ok::<(), Error>(())
						}.await {
							println!("Error in interval loop: {:?}", e);
						}
					}
				});
			},
			SwDataEventType::None => {
			}
		}

		//read values

		let dg = self.dev_group.clone();
		let id = self.id.clone();
		let dev = self.clone();
		let id_len = id.len() + 1;

		txc.send(SwMessage::AttrOwner{sender_id, name: id.clone()}).await?;

		if let Some(actions) = &self.dev_group.actions {
			//create actions for device context

			for a in actions {
				let mut a = a.clone();
				a.set_dev(&self, sender_id);
				Arc::new(a).run(system).await?;
			}
		}

		//for polled or time intervals values check state change before inform others

		let check_dup = match data_event {
			SwDataEventType::OutHttp(h) => match h.read_request {
				Some(_) => true,
				None => match &self.dev_group.data_event {
					SwDataEventType::OutHttp(h) => match &h.read_request {
						Some(_) => true,
						None => false
					},
					_ => false
				}
			},
			SwDataEventType::TimeInterval(_) => true,
			_ => false
		};

		#[allow(unreachable_code)]
		task::spawn(async move {
			if let Err(e) = async {
				let mut out_conn: SwConn = SwConn::None;

				//val last send - to resend on connection success
				let mut last_val: Option<SwValue> = None;
				let mut eh = HashMap::new();

				//id of receiver of send messages
				let mut send_sender_id = sender_id;

				//values to send on registered event
				let mut on_registered_values = Vec::new();

				loop {
					let received = rx.recv().await?;

					match received {
						SwMessage::DevData{ sender_id: _, data } => {
							if debug > Some(0) {
								println!("Device's {} received data: {:?}", id, data);
							}

							let mut ctx = SwEvalCtx{ data: Some(data), dev: Some((dev.clone(), sender_id)), ..Default::default() };
							let mut msgs = Vec::new();

							{
								let mut cvals = vals.lock().await;

								//steal vals from mutex
								let mut vs = std::mem::replace(cvals.deref_mut(), eh);

								if let Some(read) = &dg.read {
									for (name, expr) in read {

										ctx.vals = Some(vs);
										let sval = expr.eval(&mut ctx);
										vs = std::mem::take(&mut ctx.vals).unwrap();

										match sval {
											Ok(value) => {
												//only non none values - in read phrase none means "skip this attr"
												if value != SwValue::None {
													let mut changed = true;

													if check_dup {
														changed = match vs.get(name.as_str()) {
															None => true,
															Some(v) => v != &value
														}
													}

													vs.insert(name.to_string(), value.clone());

													if changed {
														msgs.push(SwMessage::DevAttr{ sender_id, name: format!("{}.{}", id, name), value: value });
													}
												}
											},
											Err(e) => {
												println!("Error getting dev {}.{} value: {} from expr {:?}", dev.id, name, e, expr);
											}
										}
									}
								}

								//give back
								eh = std::mem::replace(cvals.deref_mut(), vs);
							}

							for msg in msgs {
								txc.send(msg).await?;
							}
						},
						SwMessage::DevConn{ sender_id: _, conn } => {
							if SwConn::check_init(&out_conn) {
								out_conn = conn;

								match out_conn {
									SwConn::None => {
									},
									_ => {
										if let Some(on_connect) = &dev.dev_group.on_connect {
											let mut ctx = SwEvalCtx{ dev: Some((dev.clone(), send_sender_id)), vals_prefix: Some(dev.id.clone()), msgs: Some(Vec::new()), ..Default::default() };

											if let Err(e) = async {
												_ = on_connect.eval(&mut ctx)?;

												if let SwEvalCtx{ msgs: Some(msgs), .. } = ctx {

													if debug > Some(0) {
														println!("On connect msgs: {:?}", msgs);
													}

													//send msgs
													for msg in msgs {
														txc.send(msg).await?;
													}
												}
												Ok::<(), Error>(())
											}.await {
												println!("Error init connection of {}: {} ", dev.id, e);
											}
										}
									}
								}
							}
							else {
								out_conn = conn;
							}

							if let Some(ref v) = last_val {
								match &dev.send_value(&mut out_conn, &v, debug).await {
									Err(e) => {
										println!("Error sending lost value to {}: {}", dev.id, e);
									},
									_ => {
										last_val = None;
									}
								}
							}
						},
						SwMessage::DevRawVal{ sender_id: _, value, read_reply } => {
							//write raw data to device

							match self.get_data_event() {
								SwDataEventType::Zigbee(_) => {
									on_registered_values.push(value);
								},
								_ => {
									match dev.send_value(&mut out_conn, &value, debug).await {
										Err(e) => {
											println!("Error sending out {} raw value {}: {}", dev.id, value, e);

											if value != SwValue::None {
												last_val = Some(value);
											}
										},
										Ok(Some(data)) => {
											if read_reply {

												//resend val in case of containing current state
												txc.send(SwMessage::SendTo{ sender_id, receiver_id: sender_id, msg: Box::new(SwMessage::DevData{ sender_id, data }) }).await?;
											}
										},
										_ => {}
									}
								}
							}
						},
						SwMessage::DevAttr{ sender_id: _, name, value } => {
							//set device attr

							let mut is_write = false;

							if name.len() > id_len {
								let a_name = &name[id_len..];

								if let Some(write) = &dg.write {

									//local attrs are controlling device locally connected to system (relays on i2e, etc), we do not receive feedback from external device

									if let Some(expr) = write.get(a_name) {
										let sw;
										let mut cmsgs = None;
										is_write = true;
										//todo! check if val is really local (when use physical dev connected)

										{
											let mut cvals = vals.lock().await;

											//steal vals from mutex
											let sv = std::mem::take(cvals.deref_mut());

											//send_sender_id may be overwritten for server devices (i.e. zigbee)
											let mut ctx = SwEvalCtx{ dev: Some((dev.clone(), send_sender_id)), vals: Some(sv), msgs: Some(Vec::new()), ..Default::default() };

											ctx.val = Some(value.clone());
											ctx.data = None;

											sw = expr.eval(&mut ctx);

											//give back
											if let SwEvalCtx{ vals: Some(sv), mut msgs, .. } = ctx {
												_ = std::mem::replace(cvals.deref_mut(), sv);
												cmsgs = std::mem::take(&mut msgs);
											}
										}

										//send msgs
										if let Some(msgs) = cmsgs {
											for msg in msgs {
												txc.send(msg).await?;
											}
										}

										if let Err(e) = sw {
											println!("Error evaluating out {}.{} value {} ", dev.id, a_name, e);
										}
									}
								}

								if !is_write {
									{
										//write val to local vals
										let mut cvals = vals.lock().await;
										cvals.insert(a_name.to_string(), value.clone());
									}

									//inform observers
									txc.send(SwMessage::DevAttr{ sender_id, name, value }).await?;
								}
							}
						},
						SwMessage::ServiceRegistered{ sender_id: server_sender_id, server_id: _, server_data: _ } => {
							match self.get_data_event() {
								SwDataEventType::Zigbee(_) => {
									//devattr send messages will be send to server (no need to sending connection to device)
									send_sender_id = server_sender_id;

									//now can run on_run - sending raw values to zgate
									for value in on_registered_values {
										txc.send(SwMessage::SendTo{ sender_id, receiver_id: send_sender_id, msg: Box::new(SwMessage::DevRawVal{ sender_id, value: value, read_reply: false }) }).await?;
									}

									//for satisfy borrow checker
									on_registered_values = Vec::with_capacity(0);
								},
								_ => {
								}
							}
						},
						_ => {}
					}
				}

				Ok::<(), Error>(())
			}.await {
				println!("Error in switcher main loop: {}", e);
			}
		});

		Ok(())
	}

	async fn send_value(self: &Arc<Self>, conn: &mut SwConn, val: &SwValue, debug: Option<u8>) -> Result<Option<SwData>, Error> {
		if debug > Some(0) {
			println!("Send val to {}: {:?}", self.id, val);
		}

		let interval = if let Some(interval) = match &self.dev_group.data_event {
			SwDataEventType::OutHttp(h) => h.interval,
			SwDataEventType::TimeInterval(t) => Some(t.interval),
			_ => None
		} { interval } else { 2000 };

		//timeout to go out from hanging connection
		Ok(timeout(Duration::from_millis(interval as u64), async {

			let x = async {
				match self.get_data_event() {
					SwDataEventType::OutSocket(_) => {
						if let SwConn::Tcp{ stream: conn } = conn {
							let s = format!("{}\r\n", val);
							conn.write_all(s.as_bytes()).await?;
						}
						else {
							return Err(Error::Str("No connection while sending value"));
						}
					},
					SwDataEventType::OutHttp(h) => {
						let s = h.do_request(&self, &val, debug).await?;

						if debug > Some(0) {
							println!("Dev {} response: {:?}", self.id, s);
						}

						return Ok(Some(SwData::from_string(s)));
					},
					SwDataEventType::InMqtt(_) => {
						if let SwConn::Mqtt{ stream: conn, protocol_level } = conn {
							if let SwValue::Map(m) = val {
								if let (Some(topic), Some(payload)) = (m.get("topic"), m.get("payload")) {
									let payload = match payload {
										SwValue::String(payload) => payload,
										_ => { return Err(Error::Str("Payload has to be String")) }
									};

									let mut pp = PublishProps {
										topic_name: topic.to_string(),
										payload: payload.clone().into_bytes(),
										..Default::default()
									};

									//for mqtt 5.0
									if let Some(SwValue::Map(properties)) = m.get("properties") {
										let mut ppp = Vec::new();

										for (key, val) in properties {
											ppp.push(match key.as_str() {
												"content-type" => MqttProperty::ContentType(val.to_string()),
												"payload-format-indicator" => MqttProperty::PayloadFormatIndicator(val.clone().into_numeric()? as u8),
												k => MqttProperty::UserProperty(k.to_string(), val.to_string())
											})
										}

										pp.properties = Some(ppp);
									}

									let p = MqttPacket::Publish(pp);

									_ = p.async_write_frame(conn, protocol_level).await?;
								}
							}
						}
						else {
							return Err(Error::Str("No connection while sending value"));
						}
					},
					SwDataEventType::Zigbee(_) => {
						println!("Unproper sending value to zigbee: {:?}, {:?}!", conn, val);
					},
					_ => {}
				}

				Ok(None)
			}.await;

			Ok(x?)
		}).await?)
	}
}

#[derive(Deserialize, Debug, Clone)]
pub struct SwConnectionOutAddr {
	host: Option<String>,
	port: Option<u16>
}

#[derive(Deserialize, Default, Debug)]
#[serde(tag = "type")]
pub enum SwDataEventType {
	//time interval
	#[serde(rename = "interval")]
	TimeInterval(SwTimeInterval),
	//outgoing http connection
	#[serde(rename = "out_http")]
	OutHttp(SwConnectionOutHttp),
	//incoming mqtt connection
	#[serde(rename = "in_mqtt")]
	InMqtt(SwConnectionInMqtt),
	//outgoing socket connection
	#[serde(rename = "out_socket")]
	OutSocket(SwConnectionOutAddr),
	#[serde(rename = "zigbee")]
	Zigbee(SwZigbeeEvent),
	#[default]
	None
}

#[derive(Deserialize, Debug)]
pub struct SwTimeInterval {
	interval: u64,
	#[serde(default)]
	#[serde(with = "naive_time_format")]
	start_at: Option<NaiveTime>
}

mod naive_time_format {
	use serde::de::{ Deserializer };
	use chrono::NaiveTime;
	use serde::{ Deserialize };

	pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<NaiveTime>, D::Error>  where D: Deserializer<'de> {
		let buf = String::deserialize(deserializer)?;

		if buf.len() == 0 {
			return Ok(None);
		}

		return Ok(Some(NaiveTime::parse_from_str(&buf, "%H:%M:%S").map_err(serde::de::Error::custom)?));
	}
}

#[derive(Deserialize, Debug)]
pub struct SwConnectionOutHttp {
	host: Option<String>,
	port: Option<u16>,
	is_tls: Option<bool>,
	method: Option<String>,
	interval: Option<u64>,
	read_request: Option<SwEvaluator>,
}

impl SwConnectionOutHttp {
	pub async fn do_request(&self, dev: &SwDeviceInstance, val: &SwValue, debug: Option<u8>) -> Result<String, Error> {
		let is_tls = match self.is_tls {
			Some(is_tls) => is_tls,
			None => match &dev.dev_group.data_event {
				SwDataEventType::OutHttp(h) => match h.is_tls {
					Some(is_tls) => is_tls,
					None => false
				},
				_ => false
			}
		};

		let port = match self.port {
			Some(p) => p,
			None => match &dev.dev_group.data_event {
				SwDataEventType::OutHttp(h) => match h.port {
					Some(p) => p,
					None => if is_tls { 443 } else { 80 }
				},
				_ => if is_tls { 443 } else { 80 }
			}
		};

		let mut method = match &self.method {
			Some(m) => m,
			None => match &dev.dev_group.data_event {
				SwDataEventType::OutHttp(h) => match &h.method {
					Some(m) => m,
					None => "GET"
				},
				_ => "GET"
			}
		};

		let mut url = None;
		let mut data = None;
		let mut headers = None;
		let mut host = self.host.as_ref();

		match val {
			SwValue::String(u) => {
				url = Some(u);
			},
			SwValue::Map(m) => {
				if let Some(SwValue::String(h)) = m.get("host") {
					host = Some(h);
				}

				if let Some(SwValue::String(u)) = m.get("url") {
					url = Some(u);
				}

				if let Some(SwValue::String(m)) = m.get("method") {
					method = m;
				}

				if let Some(SwValue::String(d)) = m.get("data") {
					data = Some(d);
				}

				if let Some(SwValue::Vec(v)) = m.get("headers") {
					let mut h = HttpHeaders::new();
					let mut i = v.into_iter();

					for _ in 0..v.len() / 2 {
						if let Some(SwValue::String(key)) = i.next() {
							if let Some(SwValue::String(val)) = i.next() {
								h.header(key, val);
							}
						}
					}

					headers = Some(h);
				}
			},
			_ => {}
		}

		if let Some(host) = host {
			if let Some(url) = url {
				let s;

				let mut stream = TcpStream::connect(format!("{}:{}", host, port)).await?;

				if is_tls {
					let mut tls_stream = tls_client_connection(host, stream).await?;

					s = async_http_connection::http_request_str(method, host, port, url, headers, data.map(|x| x.as_bytes()), &mut tls_stream, debug > Some(0)).await?;
				}
				else {
					s = async_http_connection::http_request_str(method, host, port, url, headers, data.map(|x| x.as_bytes()), &mut stream, debug > Some(0)).await?;
				}

				return Ok(s);
			}
			else {
				return Err(Error::Str("Send requires url and method"));
			}
		}
		else {
			return Err(Error::Str("No host in http connection"));
		}
	}
}

#[derive(Deserialize, Debug)]
pub struct SwConnectionInMqtt {
	client_id: Option<SwEvaluator>,
	selector: Option<SwEvaluator>,
	server_id: Option<String>
}

#[derive(Deserialize, Default, Debug, Clone, PartialEq)]
pub enum SwZigbeeExtensionType {
	#[default]
	#[serde(rename = "raw")]
	Raw,
	#[serde(rename = "tuya")]
	Tuya
}

#[derive(Deserialize, Debug)]
pub struct SwZigbeeEvent {
	server_id: Option<String>,
	#[serde(default)]
	extension_type: SwZigbeeExtensionType
}

pub struct System {
	config: Config,
	run_config: serde_json::Value,
	//items observing attributes values - receive values from owners
	observers: HashMap<String, HashSet<usize>>,
	//owners of attribute - receive value from system (sender_id=0) and changes device state
	owners: HashMap<String, usize>,
	tx: Vec<Sender<SwMessage>>,
	//servers by type
	servers: HashMap<SwServerType, HashMap<String, (usize, SwServerData)>>,
	dev_groups: HashMap<String, Arc<SwDeviceType>>
}

//data from device

#[derive(Debug)]
pub enum SwZbDataValue {
	Attribute {
		identifier: u16,
		value: AttributeValue
	},
	ZclCommand(Command)
}

#[derive(Default, Debug)]
pub enum SwDataValue {
	#[default]
	None,
	Raw(Vec<u8>),
	String(String),
	Json(serde_json::value::Value),
	Zb(SwZbDataValue)
}

#[derive(Default, Debug)]
pub enum SwDataSource {
	#[default]
	None,
	Mqtt {
		topic: String
	},
	Zb {
		cluster_id: u16,
		eui64: u64
	}
}

#[derive(Default, Debug)]
pub struct SwData {
	val: SwDataValue,
	src: SwDataSource
}

impl SwData {
	pub fn from_string(s: String) -> Self {
		SwData {
			val: SwDataValue::String(s),
			..Default::default()
		}
	}

	pub fn as_str(&mut self) -> Result<&str, Error> {
		match &self.val {
			SwDataValue::String(s) => return Ok(&s.as_str()),
			_ => Err(Error::Str("SwData has no string"))
		}
	}

	pub fn as_json(&mut self) -> Result<&serde_json::value::Value, Error> {
		match &self.val {
			SwDataValue::Raw(v) => { self.val = SwDataValue::Json(serde_json::from_slice(v.as_slice())?); },
			SwDataValue::String(s) => { self.val = SwDataValue::Json(serde_json::from_str(&s)?); },
			SwDataValue::Zb(SwZbDataValue::ZclCommand(c)) => { self.val = SwDataValue::Json(c.to_json()?); },
			_ => {}
		}

		match &self.val {
			SwDataValue::Json(j) => Ok(&j),
			_ => Err(Error::Str("SwData has no JSON"))
		}
	}

	pub fn as_raw(&mut self) -> Result<&Vec<u8>, Error> {
		match &self.val {
			SwDataValue::String(s) => { self.val = SwDataValue::Raw(s.as_bytes().to_vec()); },
			SwDataValue::Zb(SwZbDataValue::ZclCommand(Command::Raw(v))) => { self.val = SwDataValue::Raw(v.to_vec()); },
			SwDataValue::Zb(SwZbDataValue::ZclCommand(c)) => { self.val = SwDataValue::Raw(c.to_bytes()?); },
			_ => {}
		}

		match &self.val {
			SwDataValue::Raw(r) => Ok(&r),
			_ => Err(Error::Str("SwData has no Raw value"))
		}
	}
}

#[derive(Eq, Hash, PartialEq, Debug)]
pub enum SwServerType {
	Http,
	Mqtt,
	Zigbee
}

#[derive(Debug)]
pub enum SwServiceType {
	MqttDev{ dev: Arc<SwDeviceInstance> },
	ZigbeeDev{ dev: Arc<SwDeviceInstance> },
	Http{ path: String, name: String },
}

impl SwServiceType {
	pub fn get_server_type(&self) -> SwServerType {
		match self {
			Self::MqttDev{ .. } => SwServerType::Mqtt,
			Self::ZigbeeDev{ .. } => SwServerType::Zigbee,
			Self::Http{ .. } => SwServerType::Http,
		}
	}
}

#[derive(Debug)]
pub enum SwServiceData {
	Http{ c: HttpConnection, conn: TcpStream }
}

#[derive(Debug)]
pub enum SwServerData {
	Http{ host: String, port: u16 },
	Zigbee,
	Mqtt{ host: String, port: u16, user_name: Option<String>, password: Option<String> },
}

impl SwServerData {
	pub fn get_server_type(&self) -> SwServerType {
		match self {
			Self::Mqtt{ .. } => SwServerType::Mqtt,
			Self::Http{ .. } => SwServerType::Http,
			Self::Zigbee{ .. } => SwServerType::Zigbee,
		}
	}
}

#[derive(Debug)]
pub enum SwConn {
	None,
	Tcp{ stream: TcpStream },
	Mqtt{ stream: TcpStream, protocol_level: u8 },
	Zigbee{ server_sender_id: usize }
}

impl Default for SwConn {
	fn default() -> Self { SwConn::None }
}

impl SwConn {
	fn check_init(conn: &Self) -> bool {
		match conn {
			//zigbee requires constatn init, because it has no "real" connection
			Self::Zigbee{ .. } |
			//no connection - init
			Self::None => true,

			//these can be disconnected
			Self::Tcp{ .. } |
			Self::Mqtt{ .. } => false,
		}
	}
}

#[derive(Debug)]
pub enum SwMessage {
	Initialized{ sender_id: usize, msg: String }, //service initialized
	DevAttr{ sender_id: usize, name: String, value: SwValue }, //switcher data value in dev.attr format
	DevRawVal{ sender_id: usize, value: SwValue, read_reply: bool }, //raw value sending to device, read_reply - is device sending reply wher reveiving new value
	ObserveAttr{ sender_id: usize, name: String}, //observe attribute
	UnobserveAttr{ sender_id: usize, name: String}, //unobserve attibutr
	AttrOwner{ sender_id: usize, name: String}, //attribute owner - manages attribute's device
	DevData{ sender_id: usize, data: SwData }, //data received from device
	DevConn{ sender_id: usize, conn: SwConn }, //device's connection
	SendTo{ sender_id: usize, receiver_id: usize, msg: Box<SwMessage> }, //send message to other service
	RegisterServer{ sender_id: usize, server_id: String, server_data: SwServerData }, //register server type (http, mqtt)
	RegisterService{ sender_id: usize, server_id: Option<String>, service_type: SwServiceType }, //register service in server type
	ServiceRegistered{ sender_id: usize, server_id: String, server_data: SwServerData }, //service registered
	Serve{ sender_id: usize, data: SwServiceData }, //serve - i.e. http request
	RunConfig{ data: serde_json::Value }	//update run config
}

#[derive(Clone, Debug)]
enum SwEvaluator {
	StdEval(Box<StdEval<Self>>),
	JsonVal{ a: Box<Self> },
	FrVal{ n: usize, a1: Box<Self>, a2: Box<Self>, a3: Box<Self> },
	Const{ a: Box<Self> },
	Date{ n: usize, a: Box<Self> },
	Time{ n: usize, a: Box<Self> },
	Duration{ n: usize, a1: Box<Self>, a2: Box<Self> },
	Topic,
	Lpad{ a1: Box<Self>, a2: Box<Self>, a3: Box<Self> },
	If{ n: usize, a1: Box<Self>, a2: Box<Self>, a3: Box<Self> },
	Substr{ n: usize, a1: Box<Self>, a2: Box<Self>, a3: Box<Self> },
	ParseNum{ a: Box<Self> },
	Scale{ a1: Box<Self>, a2: Box<Self>, a3: Box<Self>, a4: Box<Self>, a5: Box<Self> },
	Val{ n: usize, a1: Box<Self>, a2: Box<Self> },
	Dechex{ a1: Box<Self>, a2: Box<Self> },
	Strpos{ a1: Box<Self>, a2: Box<Self> },
	Day{ a1: Box<Self>, a2: Box<Self> },
	Hexdec{ a: Box<Self> },
	DecodeHex{ a: Box<Self> },
	Map{ n: usize, args: Vec<Self> },
	Now,
	Vec{ n: usize, args: Vec<Self> },
	Match{ n: usize, args: Vec<Self> },
	In{ n: usize, a: Box<Self>, args: Vec<Self> },
	SwValue(SwValue),
	Send{ n: usize, a1: Box<Self>, a2: Box<Self> },
	Semicolon{ a1: Box<Self>, a2: Box<Self> },
	Base64{ a: Box<Self> },
	ZbMatch{ n: usize, a1: Box<Self>, a2: Box<Self> },
}

impl SwEvaluator {
	fn visit(&self, v: &mut impl FnMut(&Self)) {
		v(self);

		match self {
			SwEvaluator::StdEval(s) => match s.deref() {
				StdEval::BoolNeg{ a } |
				StdEval::Neg{ a } => {
					a.visit(v);
				},
				StdEval::BoolAnd{ a1, a2 } |
				StdEval::BoolOr{ a1, a2 } |
				StdEval::BitAnd{ a1, a2 } |
				StdEval::BitOr{ a1, a2 } |
				StdEval::Eq{ a1, a2 } |
				StdEval::Neq{ a1, a2 } |
				StdEval::Add{ a1, a2 } |
				StdEval::Sub{ a1, a2 } |
				StdEval::Mul{ a1, a2 } |
				StdEval::Lt{ a1, a2 } |
				StdEval::Gt{ a1, a2 } |
				StdEval::Div{ a1, a2 } => {
					a1.visit(v);
					a2.visit(v);
				},
				StdEval::None => {}
			},
			SwEvaluator::Date{ a, .. } |
			SwEvaluator::Time{ a, .. } |
			SwEvaluator::Base64{ a } |
			SwEvaluator::Const{ a } |
			SwEvaluator::JsonVal{ a } |
			SwEvaluator::ParseNum{ a } |
			SwEvaluator::Hexdec{ a } |
			SwEvaluator::DecodeHex{ a } => {
				a.visit(v);
			},
			SwEvaluator::ZbMatch{ a1, a2, .. } |
			SwEvaluator::Dechex{ a1, a2 } |
			SwEvaluator::Day{ a1, a2 } |
			SwEvaluator::Strpos{ a1, a2 } |
			SwEvaluator::Semicolon{ a1, a2 } |
			SwEvaluator::Duration{ a1, a2, .. } |
			SwEvaluator::Send{ a1, a2, .. } |
			SwEvaluator::Val{ a1, a2, .. } => {
				a1.visit(v);
				a2.visit(v);
			},
			SwEvaluator::Lpad{ a1, a2, a3 } => {
				a1.visit(v);
				a2.visit(v);
				a3.visit(v);
			},
			SwEvaluator::In{ n: _, a, args } => {
				a.visit(v);

				for a in args {
					a.visit(v);
				}
			},
			SwEvaluator::Vec{ n: _, args } |
			SwEvaluator::Match{ n: _, args } |
			SwEvaluator::Map{ n: _, args } => {
				for a in args {
					a.visit(v);
				}
			},
			SwEvaluator::FrVal{ a1, a2, a3, .. } |
			SwEvaluator::If{ a1, a2, a3, .. } |
			SwEvaluator::Substr{ a1, a2, a3, .. } => {
				a1.visit(v);
				a2.visit(v);
				a3.visit(v);
			},
			SwEvaluator::Scale{ a1, a2, a3, a4, a5 } => {
				a1.visit(v);
				a2.visit(v);
				a3.visit(v);
				a4.visit(v);
				a5.visit(v);
			},
			SwEvaluator::Now |
			SwEvaluator::Topic => {},
			SwEvaluator::SwValue(_) => {},
		}
	}
}

impl<'de> Deserialize<'de> for SwEvaluator {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: Deserializer<'de> {
		let buf = String::deserialize(deserializer)?;

		let mut t: Tokenizer<SwEvaluator> = Tokenizer::new(&buf);
		t.string_marker = '\'';
		t.ast().map_err(serde::de::Error::custom)
	}
}

impl From<SwValue> for SwEvaluator {
	fn from (value: SwValue) -> Self {
		SwEvaluator::SwValue(value)
	}
}

impl Default for SwEvaluator {
	fn default() -> SwEvaluator {
		SwEvaluator::SwValue(SwValue::None)
	}
}

#[derive(Default)]
pub struct SwEvalCtx {
	val: Option<SwValue>,
	vals: Option<HashMap<String, SwValue>>,
	vals_prefix: Option<String>,
	data: Option<SwData>,
	dev: Option<(Arc<SwDeviceInstance>, usize)>,
	msgs: Option<Vec<SwMessage>>
}

impl Eval for SwEvaluator {
	type EvalRes = SwValue;
	type EvalCtx = SwEvalCtx;
	type Error = Error;

	fn match_str(s: &str, ahead: bool) -> Option<(Self, usize)> {
		if let Some((se, l)) = StdEval::match_str(s, ahead) {
			return Some((SwEvaluator::StdEval(Box::new(se)), l));
		}

		let l = s.len();

		if l >= 1 {
			match &s[..1] {
				";" => { return Some((SwEvaluator::Semicolon{ a1: Self::def(), a2: Self::def() }, 1)); },
				_ => {
					if l >= 2 {
						match &s[..2] {
							"if" => { return Some((SwEvaluator::If{ n: 3, a1: Self::def(), a2: Self::def(), a3: Self::def() }, 2)); },
							"in" => { return Some((SwEvaluator::In{ n: 0, a: Self::def(), args: Vec::new() }, 2)); },
							_ => {
								if l >= 3 {
									match &s[..3] {
										"val" => { return Some((SwEvaluator::Val{ n: 0, a1: Self::def(), a2: Self::def() }, 3)); },
										"map" => { return Some((SwEvaluator::Map{ n: 0, args: Vec::new() }, 3)); },
										"vec" => { return Some((SwEvaluator::Vec{ n: 0, args: Vec::new() }, 3)); },
										"now" => { return Some((SwEvaluator::Now, 3)); },
										"day" => { return Some((SwEvaluator::Day{ a1: Self::def(), a2: Self::def() }, 3)); },
										_ => {
											if l >= 4 {
												match &s[..4] {
													"date" => { return Some((SwEvaluator::Date{ n: 0, a: Self::def() }, 4)); },
													"time" => { return Some((SwEvaluator::Time{ n: 0, a: Self::def() }, 4)); },
													"lpad" => { return Some((SwEvaluator::Lpad{ a1: Self::def(), a2: Self::def(), a3: Self::def() }, 4)); },
													"send" => { return Some((SwEvaluator::Send{ n: 0, a1: Self::def(), a2: Self::def() }, 4)); },
													_ => {
														if l >= 5 {
															match &s[..5] {
																"const" => { return Some((SwEvaluator::Const{ a: Self::def() }, 5)); },
																"match" => { return Some((SwEvaluator::Match{ n: 0, args: Vec::new() }, 5)); },
																"topic" => { return Some((SwEvaluator::Topic, 5)); },
																"scale" => { return Some((SwEvaluator::Scale{ a1: Self::def(), a2: Self::def(), a3: Self::def(), a4: Self::def(), a5: Self::def() }, 5)); },
																_ => {
																	if l >= 6 {
																		match &s[..6] {
																			"base64" => { return Some((SwEvaluator::Base64{ a: Self::def() }, 6)); },
																			"substr" => { return Some((SwEvaluator::Substr{ n: 3, a1: Self::def(), a2: Self::def(), a3: Self::def() }, 6)); },
																			"strpos" => { return Some((SwEvaluator::Strpos{ a1: Self::def(), a2: Self::def() }, 6)); },
																			"dechex" => { return Some((SwEvaluator::Dechex{ a1: Self::def(), a2: Self::def() }, 6)); },
																			"hexdec" => { return Some((SwEvaluator::Hexdec{ a: Self::def() }, 6)); },
																			"fr_val" => { return Some((SwEvaluator::FrVal{ n: 0, a1: Self::def(), a2: Self::def(), a3: Self::def() }, 6)); },
																			_ => {
																				if l >= 8 {
																					match &s[..8] {
																						"json_val" => { return Some((SwEvaluator::JsonVal{ a: Self::def() }, 8)); },
																						"duration" => { return Some((SwEvaluator::Duration{ n: 0, a1: Self::def(), a2: Self::def() }, 8)); },
																						"zb_match" => { return Some((SwEvaluator::ZbMatch{ n: 0, a1: Self::def(), a2: Self::def() }, 8)); },
																						_ => {
																							if l >= 9 {
																								match &s[..9] {
																									"parse_num" => { return Some((SwEvaluator::ParseNum{ a: Self::def() }, 9)); },
																									_ => {
																										if l >= 10 {
																											match &s[..10] {
																												"decode_hex" => { return Some((SwEvaluator::DecodeHex{ a: Self::def() }, 10)); },
																												_ => {}
																											}
																										}
																									}
																								}
																							}
																						}
																					}
																				}
																			}
																		}
																	}
																}
															}
														}
													}
												}
											}
										}
									}
								}
							}
						}
					}
				}
			}
		}

		if !ahead {
			match s {
				"none" => { return Some((Self::SwValue(SwValue::None), l)); },
				"true" => { return Some((Self::SwValue(true.into()), l)); },
				"false" => { return Some((Self::SwValue(false.into()), l)); },
				_ => {
					if let Ok(n) = s.parse::<i32>() {
						return Some((Self::SwValue(n.into()), l));
					}
					else {
						if let Ok(f) = s.parse::<f32>() {
							return Some((Self::SwValue(f.into()), l));
						}
					}
				}
			}
		}

		None
	}

	fn get_token_type(&self) -> TokenType {
		match self {
			SwEvaluator::StdEval(s) => s.get_token_type(),
			SwEvaluator::SwValue(_) => TokenType::Literal,
			SwEvaluator::In{ .. } => TokenType::Operator{ precedence: 15, assoc: tokenizer::Assoc::Left },
			SwEvaluator::Semicolon{ .. } => TokenType::Operator{ precedence: 1, assoc: tokenizer::Assoc::Left },
			_ => TokenType::Function
		}
	}

	fn set_arg_num(&mut self, num: usize) {
		match self {
			SwEvaluator::If{ n, .. } |
			SwEvaluator::Match{ n, .. } |
			SwEvaluator::Val{ n, .. } |
			SwEvaluator::In{ n, .. } |
			SwEvaluator::Map{ n, .. } |
			SwEvaluator::Vec{ n, .. } |
			SwEvaluator::Send{ n, .. } |
			SwEvaluator::ZbMatch{ n, .. } |
			SwEvaluator::FrVal{ n, .. } |
			SwEvaluator::Duration{ n, .. } |
			SwEvaluator::Substr{ n, .. } => { *n = num; },
			_ => {}
		}
	}

	fn build(&mut self, tokens: &mut Vec<Token<Self>>) -> Result<(), Self::Error> {
		match self {
			SwEvaluator::StdEval(s) => { s.build(tokens)?; },
			SwEvaluator::Const{ a } |
			SwEvaluator::Base64{ a } |
			SwEvaluator::JsonVal{ a } |
			SwEvaluator::ParseNum{ a } |
			SwEvaluator::DecodeHex{ a } |
			SwEvaluator::Hexdec{ a } => {
				*a = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
			},
			SwEvaluator::Lpad{ a1, a2, a3 } => {
				*a3 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
				*a2= Box::new(Tokenizer::<Self>::get_expr(tokens)?);
				*a1= Box::new(Tokenizer::<Self>::get_expr(tokens)?);
			},
			SwEvaluator::Date{ n, a } |
			SwEvaluator::Time{ n, a } => {
				if *n > 0 {
					*a = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
				}
			},
			SwEvaluator::Day{ a1, a2 } |
			SwEvaluator::Strpos{ a1, a2 } |
			SwEvaluator::Dechex{ a1, a2 } |
			SwEvaluator::Semicolon{ a1, a2 } => {
				*a2 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
				*a1 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
			},
			SwEvaluator::ZbMatch{ n, a1, a2 } |
			SwEvaluator::Send{ n, a1, a2 } |
			SwEvaluator::Val{ n, a1, a2 } => {
				if *n == 2 {
					*a2 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
				}

				if *n > 0 {
					*a1 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
				}
			},
			SwEvaluator::In{ n, a, args } => {
				for _ in 0..*n {
					args.push(Tokenizer::<Self>::get_expr(tokens)?);
				}

				*a = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
			},
			SwEvaluator::Vec{ n, args } |
			SwEvaluator::Match{ n, args } |
			SwEvaluator::Map{ n, args } => {
				for _ in 0..*n {
					args.push(Tokenizer::<Self>::get_expr(tokens)?);
				}

				//args are in reverse order
				args.reverse();
			},
			SwEvaluator::Duration{ n, a1, a2 } => {
				if *n > 1 {
					*a2 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
				}
				else {
					*a2 = Box::new(SwEvaluator::Now);
				}

				*a1 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
			},
			SwEvaluator::If{ n, a1, a2, a3 } |
			SwEvaluator::FrVal{ n, a1, a2, a3 } |
			SwEvaluator::Substr{ n, a1, a2, a3 } => {
				if *n > 2 {
					*a3 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
				}

				*a2 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
				*a1 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
			},
			SwEvaluator::Scale{ a1, a2, a3, a4, a5 } => {
				*a5 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
				*a4 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
				*a3 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
				*a2 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
				*a1 = Box::new(Tokenizer::<Self>::get_expr(tokens)?);
			},
			_ => {}
		}

		Ok(())
	}

	fn eval(&self, ctx: &mut Self::EvalCtx) -> Result<Self::EvalRes, Self::Error> {
		match self {
			SwEvaluator::StdEval(s) => {
				return s.eval(ctx);
			},
			SwEvaluator::JsonVal{ a } => {
				if let Ok(path) = a.eval(ctx)?.into_string() {
					match ctx {
						SwEvalCtx{ data: Some(d), .. } => {
							match d.as_json() {
								Ok(j) => {
									match get_json_from_path(j, &path) {
										Some(serde_json::value::Value::Bool(b)) => { return Ok(SwValue::Bool(*b)); },
										Some(serde_json::value::Value::Number(n)) => { return if n.is_i64() { Ok(SwValue::Numeric(n.as_i64().unwrap() as i32)) } else { Ok(SwValue::Float(n.as_f64().unwrap() as f32)) }; }
										Some(serde_json::value::Value::String(s)) => { return Ok(SwValue::String(s.clone())); },
										_ => {}
									}
								},
								Err(e) => { return Err(Error::String(format!("json_val() requires json data in context: {}", e))); }
							}
						},
						_ => {
							return Err(Error::Str("json_val() requires json data in context"));
						}
					}
				}
			},
			SwEvaluator::ZbMatch{ n, a1, a2 } => {
				let d_cluster_id = a1.eval(ctx)?.into_numeric()? as u16;

				match n {
					1 => {
						if let SwEvalCtx{ data: Some(SwData{ src: SwDataSource::Zb{ cluster_id, .. }, ..}), ..} = ctx {
							return Ok(SwValue::Bool(d_cluster_id == *cluster_id));
						}
					},
					2 => {
						let d_identifier = a2.eval(ctx)?.into_numeric()? as u16;

						if let SwEvalCtx{ data: Some(SwData{ src: SwDataSource::Zb{ cluster_id, .. }, val: SwDataValue::Zb(SwZbDataValue::Attribute{ identifier, .. })}), ..} = ctx {
							return Ok(SwValue::Bool(d_cluster_id == *cluster_id && d_identifier == *identifier));
						}
					},
					_ => {}
				}

				return Err(Error::Str("zb_match() requires zigbee data in context"));
			},
			SwEvaluator::FrVal{ n, a1, a2, a3 } => {
				//starting position in byte array
				let mut pos = a1.eval(ctx)?.into_numeric()? as usize;

				//length of multibyte value, default 1
				let len = if *n < 2 { 1 } else { a2.eval(ctx)?.into_numeric()? as usize };

				//is little endian?, default false
				let is_le = if *n < 3 { false } else { a3.eval(ctx)?.into_bool()? };

				if is_le {
					pos += len;
				}

				match ctx {
					SwEvalCtx{ data: Some(d), .. } => {
						match d.as_raw() {
							Ok(r) => {
								let mut v: i32 = 0;

								for _ in 0..len {
									v = v << 8;

									if let Some(b) = r.get(pos) {
										v += *b as i32;
									}

									if is_le {
										pos -= 1;
									}
									else {
										pos += 1;
									}
								}

								return Ok(SwValue::Numeric(v));
							},
							Err(e) => { return Err(Error::String(format!("fr_val() requires raw data in context: {}", e))); }
						}
					},
					_ => {}
				}

				return Err(Error::Str("fr_val() requires raw data in context"));
			},
			SwEvaluator::Base64{ a } => {
				let val = a.eval(ctx)?.into_string()?;
				let b64 = general_purpose::STANDARD_NO_PAD.encode(&val.as_str());

				return Ok(SwValue::String(b64));
			},
			SwEvaluator::Val{ n, a1, a2 } => {
				match *n {
					0 => {
						//get currernt val

						match ctx {
							SwEvalCtx{ data: Some(d), .. } => {
								if let SwData{ val: SwDataValue::Zb(SwZbDataValue::Attribute{ value: av, .. }), .. } = d {
									return Ok(SwValue::from(av.clone()));
								}

								match d.as_str() {
									Ok(d) => { return Ok(SwValue::String(d.to_string())); },
									Err(e) => { return Err(Error::String(format!("val() requires string data in context: {}", e))); }
								}
							},
							SwEvalCtx{ val: Some(v), .. } => {
								return Ok(v.clone());
							},
							_ => {
								return Err(Error::Str("val() requires string or zigbee data in context"));
							}
						}
					},
					1 => {
						//get named attribute val or login val (session or token)

						let name = a1.eval(ctx)?;

						match ctx {
							SwEvalCtx{ vals: Some(vals), vals_prefix, .. } => {
								//vals could be current device vals in device instance
								//or global vals in action

								let mut name = name.into_string()?;

								if let Some(vals_prefix) = vals_prefix {
									name = format!("{}.{}", vals_prefix, name);
								}

								match vals.get(&name) {
									Some(v) => { return Ok(v.clone()); },
									None => { return Ok(SwValue::None); }
								}
							},
							_ => {
								return Err(Error::Str("val(name) requires dev data in context"));
							}
						}
					},
					2 => {
						//set named val

						let mut name = a1.eval(ctx)?.into_string()?;
						let value = a2.eval(ctx)?;

						match ctx {
							SwEvalCtx{ msgs: Some(msgs), vals_prefix, .. } => {

								if let Some(vals_prefix) = vals_prefix {
									name = format!("{}.{}", vals_prefix, name);
								}

								msgs.push(SwMessage::DevAttr{ sender_id: 0, name, value });
							},
							_ => {
								return Err(Error::Str("val(name, value) requires msgs in context"));
							}
						}
					},
					_ => {
						return Err(Error::Str("val requires at most 3 parameters"));
					}
				}
			},
			SwEvaluator::Semicolon{ a1, a2 } => {
				a1.eval(ctx)?;

				return Ok(a2.eval(ctx)?);
			},
			SwEvaluator::Send{ n, a1, a2 } => {
				let vals;
				let read_reply = *n == 2 && a2.eval(ctx)?.into_bool()?;

				match a1.eval(ctx)? {
					SwValue::Vec(v) => {
						vals = v;
					},
					a => {
						vals = vec![a];
					}
				}

				match ctx {
					SwEvalCtx{ dev: Some((_, service_id)), msgs: Some(msgs), .. } => {
						for val in vals {
							msgs.push(SwMessage::SendTo{
								sender_id: 0,
								receiver_id: *service_id,
								msg: Box::new(SwMessage::DevRawVal{
									sender_id: 0,
									value: val,
									read_reply
								})
							});
						}
					},
					_ => {
						return Err(Error::Str("send() requires msgs and dev in context"));
					}
				}
			},
			SwEvaluator::Const{ a } => {
				match a.eval(ctx) {
					Ok(name) => {
						match ctx {
							SwEvalCtx{ dev: Some((dev, _)), .. } => {
								if let Some(consts) = &dev.consts {
									if let Some(c) = consts.get(&name.into_string()?) {
										return Ok(c.clone());
									}
								}
							},
							_ => {
								return Err(Error::Str("const() requires dev in context"));
							}
						}
					},
					Err(e) => {
						return Err(e);
					}
				}

				return Ok(SwValue::None);
			},
			SwEvaluator::Topic => {
				match ctx {
					SwEvalCtx{ data: Some(SwData { src: SwDataSource::Mqtt{ topic }, .. }), .. } => {
						return Ok(SwValue::String(topic.clone()));
					},
					_ => {
						return Err(Error::Str("topic() requires mqtt topic in context"));
					}
				}
			},
			SwEvaluator::Day{ a1, a2 } => {
				let now = Local::now();
				let (sunrise, sunset) = sunrise::sunrise_sunset(a1.eval(ctx)?.into_float()? as f64, a2.eval(ctx)?.into_float()? as f64, now.year(), now.month(), now.day());

				let now_t = now. timestamp();

				return Ok(SwValue::Bool(now_t >= sunrise && now_t < sunset));
			},
			SwEvaluator::Lpad{ a1, a2, a3 } => {
				let mut s = a1.eval(ctx)?.into_string()?;
				let l = a2.eval(ctx)?.into_numeric()?;
				let p = a3.eval(ctx)?.into_string()?;

				while s.len() < l as usize {
					//todo optimalization
					s = p.clone() + &s;
				}

				return Ok(SwValue::String(s));
			},
			SwEvaluator::If{ n, a1, a2, a3 } => {
				return Ok(match a1.eval(ctx)?.into_bool()? {
					true => a2.eval(ctx)?,
					false => if *n < 3 { SwValue::None } else { a3.eval(ctx)? }
				});
			},
			SwEvaluator::Substr{ n, a1, a2, a3 } => {
				let r = a1.eval(ctx)?;
				let s = r.into_string()?;

				let len = s.len();

				let f = a2.eval(ctx)?.into_numeric()? as usize;
				let l = if *n < 3 { len - f } else { a3.eval(ctx)?.into_numeric()? as usize };

				if f >= len {
					return Ok(SwValue::String("".to_string()));
				}
				else {
					return Ok(SwValue::String(s[f..min(len, f + l)].to_string()));
				}
			},
			SwEvaluator::Strpos{ a1, a2 } => {
				let h = a1.eval(ctx)?.into_string()?;
				let n = a2.eval(ctx)?.into_string()?;

				match h.find(&n) {
					Some(pos) => return Ok(SwValue::Numeric(pos as i32)),
					None => return Ok(SwValue::None)
				}
			},
			SwEvaluator::ParseNum{ a } => {
				let r = a.eval(ctx)?;
				let s = r.into_string()?;
				if let Ok(n) = s.parse::<i32>() {
					return Ok(SwValue::Numeric(n));
				}
				else {
					return Err(Error::String(format!("parse_num() error parsing {}", s)));
				}
			},
			SwEvaluator::Scale{ a1, a2, a3, a4, a5 } => {
				let value = a1.eval(ctx)?;

				if let SwValue::None = value {
					return Ok(SwValue::None);
				}

				let value = value.into_numeric()?;
				let min_value = a2.eval(ctx)?.into_numeric()?;
				let max_value = a3.eval(ctx)?.into_numeric()?;
				let out_min_value = a4.eval(ctx)?.into_numeric()?;
				let out_max_value = a5.eval(ctx)?.into_numeric()?;

				return Ok(SwValue::Numeric((value - min_value) * (out_max_value - out_min_value) / (max_value - min_value) + out_min_value));
			},
			SwEvaluator::DecodeHex{ a } => {
				match a.eval(ctx)?.into_string()? {
					s => { if let Ok(v) = sw_device::decode_hex(&s) { return Ok(SwValue::String(String::from_utf8_lossy(&v).trim().to_string())); } },
				}
			},
			SwEvaluator::Dechex{ a1, a2 } => {
				let len = a2.eval(ctx)?.into_numeric()? as usize;
				match a1.eval(ctx)?.into_numeric()? {
					n => { return Ok(SwValue::String(format!("{:01$x}", n, len))); },
				}
			},
			SwEvaluator::Hexdec{ a } => {
				match a.eval(ctx)?.into_string()? {
					s => { if let Ok(v) = i32::from_str_radix(&s, 16) { return Ok(SwValue::Numeric(v)); } },
				}
			},
			SwEvaluator::SwValue(v) => { return Ok(v.clone()); },
			SwEvaluator::Map{ n, args } => {
				let mut m = HashMap::new();
				let mut i = args.iter();

				for _ in 0..n/2 {
					if let (Some(k), Some(v)) = (i.next(), i.next()) {
						m.insert(k.eval(ctx)?.into_string()?, v.eval(ctx)?);
					}
				}

				return Ok(SwValue::Map(m));
			},
			SwEvaluator::Vec{ args, .. } => {
				let mut v = Vec::new();

				for val in args.iter() {
					v.push(val.eval(ctx)?);
				}

				return Ok(SwValue::Vec(v));
			},
			SwEvaluator::Match{ n, args } => {
				let mut i = args.iter();

				if let Some(v) = i.next() {
					let v = v.eval(ctx)?;

					for _ in 0..n/2 {
						if let (Some(cv), Some(ov)) = (i.next(), i.next()) {
							if v == cv.eval(ctx)? {
								return Ok(ov.eval(ctx)?);
							}
						}
					}
				}
			},
			SwEvaluator::Date{ n, a } => {
				let f = if *n == 0 { "%Y-%m-%d".to_string() } else { a.eval(ctx)?.into_string()? };
				let now: DateTime<Local> = Local::now();

				return Ok(SwValue::String(now.format(&f).to_string()));
			},
			SwEvaluator::Time{ n, a } => {
				let f = if *n == 0 { "%H:%M:%S".to_string() } else { a.eval(ctx)?.into_string()? };
				let now: DateTime<Local> = Local::now();

				return Ok(SwValue::String(now.format(&f).to_string()));
			},
			SwEvaluator::Now => {
				let now: DateTime<Local> = Local::now();

				return Ok(SwValue::Numeric(now.timestamp() as i32));
			},
			SwEvaluator::Duration{ a1, a2, .. } => {
				let to = a2.eval(ctx)?.into_numeric()?;
				let from = a1.eval(ctx)?.into_numeric()?;

				return Ok(SwValue::Numeric(to - from));
			},
			SwEvaluator::In{ a, args, .. } => {
				let v = a.eval(ctx)?;

				for val in args.iter() {
					if v == val.eval(ctx)? {
						return Ok(SwValue::Bool(true));
					}
				}

				return Ok(SwValue::Bool(false));
			}
		}

		Ok(SwValue::None)
	}
}

impl System {
	pub fn new(c: Config) -> Self {
		let mut run_config = None;

		if let Ok(f) = Self::get_run_config_file(&c) {
			if let Ok(v) = serde_json::from_reader(f) {
				run_config = Some(v);
			}
		}

		System {
			config: c,
			run_config: run_config.unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
			observers: HashMap::new(),
			owners:  HashMap::new(),
			tx: Vec::new(),
			servers: HashMap::new(),
			dev_groups: HashMap::new()
		}
	}

	fn get_run_config_file(c: &Config) -> Result<File, Error> {
		let mut p = match &c.data_dir {
			None => std::env::current_dir()?.into_os_string().into_string().unwrap(),
			Some(s) => s.to_string()
		};

		p.push_str("/run_config.json");

		Ok(File::options().read(true).write(true).create(true).open(p)?)
	}

	fn merge_json(a: &mut serde_json::Value, b: serde_json::Value) {
		if let serde_json::Value::Object(a) = a {
			if let serde_json::Value::Object(b) = b {
				for (k, v) in b {
					if v.is_null() {
						a.remove(&k);
					}
					else {
						Self::merge_json(a.entry(k).or_insert(serde_json::Value::Null), v);
					}
				}

				return;
			}
		}

		*a = b;
	}

	pub fn upd_run_cfg(&mut self, b: serde_json::Value) -> Result<(), Error> {
		Self::merge_json(&mut self.run_config, b);

		//save modified config
		let mut f = Self::get_run_config_file(&self.config)?;

		if self.config.debug > Some(0) {
			println!("update run config: {:?}", self.run_config);
		}

		f.set_len(0)?;
		serde_json::to_writer(&mut f, &self.run_config)?;

		Ok(())
	}

	fn get_dev_group(&self, group: &str) -> Option<Arc<SwDeviceType>> {
		return self.dev_groups.get(group).map(|dg| dg.clone());
	}

	fn get_channel(&mut self) -> (usize, Receiver<SwMessage>, Sender<SwMessage>) {
		let tx_e = self.tx[0].clone();
		let (tx_s, rx_e) = unbounded();
		let sender_id = self.tx.len();

		self.tx.push(tx_s);

		(sender_id, rx_e, tx_e)
	}

	fn get_server(&self, server_type: SwServerType, server_id: Option<&String>) -> Option<&(usize, SwServerData)>
	{
		match self.servers.get(&server_type) {
			Some(servers) => {
				match server_id {
					Some(server_id) => servers.get(server_id),
					None => if servers.len() == 1 {
						servers.values().next()
					}
					else {
						None
					}
				}
			},
			None => None
		}
	}

	//main system's message loop

	#[allow(unreachable_code)]
	pub async fn run(&mut self) -> Result<(), Error> {
		let (tx_e, rx) = unbounded();
		self.tx.push(tx_e);

		//new type devices

		for s in &std::mem::take(&mut self.config.servers) {
			s.run(self).await?;
		}

		let cfg_device_groups = std::mem::take(&mut self.config.device_groups);

		for (name, dev_group) in cfg_device_groups.into_iter() {
			self.dev_groups.insert(name, Arc::new(dev_group));
		}

		for mut di in std::mem::take(&mut self.config.device_instances) {
			if let Some(dg) = self.get_dev_group(&di.group) {
				di.dev_group = dg;
			}

			Arc::new(di).run(self).await?;
		}

		for e in &std::mem::take(&mut self.config.services) {
			e.run(self).await?;
		}

		//actions
		for a in std::mem::take(&mut self.config.actions) {
			Arc::new(a).run(self).await?;
		}

		loop {
			let received = rx.recv().await?;

			match received {
				SwMessage::RunConfig{ data } => {
					self.upd_run_cfg(data)?;
				},
				SwMessage::AttrOwner{sender_id, name} => {
					if let None = self.owners.get(&name) {
						self.owners.insert(name, sender_id);
					}
				},
				SwMessage::ObserveAttr{sender_id, name} => {
					match self.observers.get_mut(&name) {
						Some(s) => {
							s.insert(sender_id);
						},
						None => {
							let mut s = HashSet::new();
							s.insert(sender_id);
							self.observers.insert(name, s);
						}
					}
				},
				SwMessage::UnobserveAttr{sender_id, name} => {
					if let Some(s) = self.observers.get_mut(&name) {
						s.remove(&sender_id);
					}
				},
				SwMessage::DevAttr{sender_id, name, value} => {
					let dot_pos = name.find('.');

					match value {
						//SwValue::None => {},
						value => {
							let mut inform_observers = true;

							if let Some(dot_pos) = dot_pos {
								//never check if value changed!
								//some devices has delay in state refresh
								//hue is sending power on even in only color change - todo!

								if sender_id == 0 {
									//send value to owner
									if let Some(owner_id) = self.owners.get(&name[0..dot_pos]) {
										if let Some(o) = self.tx.get(*owner_id) {
											if self.config.debug > Some(0) {
												println!("Sending value {} to device owner {}", value, name);
											}

											//device will resend value when get ack from get response from outer dev,
											//or when attr is local
											inform_observers = false;

											o.send(SwMessage::DevAttr{ sender_id, name: name.clone(), value: value.clone() }).await?;
										}
									}
								}
								else {
									if self.config.debug > Some(0) {
										println!("Device {} changed value to {}", name, value);
									}
								}
							}

							if inform_observers {
								//inform observers - attr without owner or from owner

								for o in vec![self.observers.get("#"), self.observers.get(&name)] {
									if let Some(s) = o {
										for observer_id in s.iter() {
											if *observer_id != sender_id {
												if let Some(o) = self.tx.get(*observer_id) {
													o.send(SwMessage::DevAttr{sender_id, name: name.clone(), value: value.clone()}).await?;
												}
											}
										}
									}
								}
							}
						}
					}
				},
				SwMessage::Initialized{ sender_id: _, msg} => {
					println!("{}", msg);
				},
				SwMessage::DevData{ sender_id: _, data } => {
					println!("unknown data: {:?}", data);
				},
				SwMessage::SendTo{ sender_id: _, receiver_id, msg } => {
					//forward message
					if let Some(tx) = self.tx.get(receiver_id) {
						tx.send(*msg).await?;
					}
				},
				SwMessage::RegisterServer{ sender_id, server_id, server_data } => {
					let server_type = server_data.get_server_type();
					self.servers.entry(server_type).or_default().insert(server_id, (sender_id, server_data));
				},
				SwMessage::RegisterService{ sender_id, server_id, service_type } => {
					let server_type = service_type.get_server_type();

					//register on server
					if let Some((server_sender_id, _)) = self.get_server(server_type, server_id.as_ref()) {
						if let Some(tx) = self.tx.get(*server_sender_id) {
							//forward message to server
							tx.send(SwMessage::RegisterService{ sender_id, server_id, service_type }).await?;
						}
					}
				},
				_ => {}
			}
		}

		Ok(())
	}
}
