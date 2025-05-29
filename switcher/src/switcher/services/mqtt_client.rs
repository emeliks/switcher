use serde::{ Deserialize };
use std::{ time::Duration, net::Shutdown };
use net_services::{ compat::AsyncWrite };
use mqtt::mqtt_packet::{ self, MqttPacket, ConnectProps, SubscribeProps, SubscribeTopic };
use crate::{ switcher::{ SwMessage, System, sw_device::{ Error } } };
use crate::compat::{ net::TcpStream, task, future::timeout };
use frames::Frame;

//mqtt client connnecting both to outer broker and local mqttServer
#[derive(Deserialize, Debug)]
pub struct SwServerMqttClient {
	id: String,
	ip: String,
	topics: Vec<String>,
	port: Option<u16>,
	protocol_level: Option<u8>,
	user_name: Option<String>,
	password: Option<String>,
	keep_alive: Option<u16>,
	local_ip: Option<String>,
	local_port: Option<u16>
}

impl SwServerMqttClient {
	pub async fn run(&self, system: &mut System) -> Result<(), Error> {
		let (sender_id, _, tx) = system.get_channel();

		let out_addr = format!("{}:{}", self.ip, match self.port { Some(port) => port, None => 1883 });
		let protocol_level = match self.protocol_level { Some(protocol_level) => protocol_level, None => 4 };
		let keep_alive = match self.keep_alive { Some(keep_alive) => keep_alive, None => 36000 };
		let local_addr = format!("{}:{}", match &self.local_ip { Some(local_ip) => local_ip, None => &system.config.ip }, match &self.local_port { Some(local_port) => *local_port, None => 1883 });

		let local_user_name = None;
		let local_password = None;

		let out_cp = MqttPacket::Connect(ConnectProps{
			protocol_name: "MQTT".to_string(),
			protocol_level,
			keep_alive,	//secs
			client_identifier: self.id.clone(),
			user_name: self.user_name.clone(),
			password: self.password.clone(),
			..Default::default()
		});

		let topics = self.topics.iter().map(|t| { SubscribeTopic {
			topic_name: t.clone(),
			..Default::default()
		}}).collect();

		let out_sp = MqttPacket::Subscribe(SubscribeProps {
			packet_identifier: 1,
			topics,
			properties: None
		});

		let local_cp = MqttPacket::Connect(ConnectProps{
			protocol_name: "MQTT".to_string(),
			protocol_level,
			keep_alive,	//secs
			client_identifier: self.id.clone(),
			user_name: local_user_name,
			password: local_password,
			..Default::default()
		});

		task::spawn(async move {
			loop {
				#[allow(unreachable_code)]
				if let Err(e) = async {
					let mut out_stream = TcpStream::connect(&out_addr).await?;
					let mut out_stream_c = out_stream.clone();
					let mut local_stream = TcpStream::connect(&local_addr).await?;

					local_cp.async_write_frame(&mut local_stream, &protocol_level).await?;
					_ = MqttPacket::async_read_frame(&mut local_stream, &protocol_level).await?;

					out_cp.async_write_frame(&mut out_stream, &protocol_level).await?;
					_ = MqttPacket::async_read_frame(&mut out_stream, &protocol_level).await?;

					tx.send(SwMessage::Initialized{sender_id, msg: format!("Connected to local mqtt server on addr {}, keepalive {}, protocol {}", &local_addr, keep_alive, protocol_level)}).await?;

					out_sp.async_write_frame(&mut out_stream, &protocol_level).await?;
					_ = MqttPacket::async_read_frame(&mut out_stream, &protocol_level).await?;

					tx.send(SwMessage::Initialized{sender_id, msg: format!("Connected to mqtt broker on addr {}, keepalive {}, protocol {}", &out_addr, keep_alive, protocol_level)}).await?;

					//mqtt broker -> local mqtt

					let mut local_stream_c = local_stream.clone();

					task::spawn(async move {
						if let Err(e) = async {
							loop {
								let p = MqttPacket::read_buf(&mut out_stream, &protocol_level).await?;

								match mqtt_packet::get_packet_type(&p) {
									//Publish Puback Pingreq Pingresp
									3 | 4 | 12 | 13 => {
										local_stream_c.write_all(&p).await?;
									},
									//Disconnect
									14 => break,
									_ => {}
								}
							}

							Ok::<(), Error>(())
						}.await {
							println!("Error in mqtt outer broker read loop: {:?}", e);

							//closing both streams
							_ = out_stream.shutdown(Shutdown::Both);
							_ = local_stream_c.shutdown(Shutdown::Both);
						}
					});

					//local mqtt -> mqtt broker

					if let Err(e) = async {
						loop {
							//in case of no operations longer that keep_alive time - send ping to outer broker

							let rf = MqttPacket::read_buf(&mut local_stream, &protocol_level);

							let p = if keep_alive <= 1 {
								rf.await?
							}
							else {
								match timeout(Duration::from_secs((keep_alive - 1) as u64), rf).await {
									Ok(pe) => pe,
									Err(_) => {
										let mut buf = Vec::new();
										MqttPacket::Pingreq.as_bytes(protocol_level, &mut buf)?;

										Ok(buf)
									}
								}?
							};

							match mqtt_packet::get_packet_type(&p) {
								//Publish Puback Pingreq Pingresp
								3 | 4 | 12 | 13 => {
									out_stream_c.write_all(&p).await?;
								},
								//Disconnect
								14 => break,
								_ => {}
							}
						}

						Ok::<(), Error>(())
					}.await {
							println!("Error in mqtt local read loop: {:?}", e);

						//closing both streams
						_ = out_stream_c.shutdown(Shutdown::Both);
						_ = local_stream.shutdown(Shutdown::Both);
					}

					Ok::<(), Error>(())
				}.await {
					println!("Error in mqtt client loop: {:?}", e);
				}

				println!("Waiting 5 sec");
				task::sleep(Duration::from_secs(5)).await;
			}
		});

		Ok(())
	}
}
