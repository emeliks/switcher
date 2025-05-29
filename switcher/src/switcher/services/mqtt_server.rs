use serde::{ Deserialize };
use crate::compat::{ sync::{ Mutex }, net::{ TcpListener }, task };
use std::{ sync::Arc, collections::{ HashMap, HashSet }, str::FromStr, net::Shutdown, time::Duration };
use futures::stream::StreamExt;
use tokenizer::Eval;
use mqtt::{ mqtt_packet::{ self, MqttPacket, PublishProps, MqttProperty, SubackProps, ConnackReturnCode, ConnackProps, SubackRet, UnsubackProps, PubackProps } };
use crate::{ switcher::{ SwMessage, System, sw_device::{ Error }, SwConn, SwServerData, SwEvalCtx, SwDataEventType, SwValue, SwDeviceInstance, SwServiceType, SwData, SwDataValue, SwDataSource } };
use frames::Frame;

#[derive(Deserialize, Debug)]
pub struct SwServerMqtt {
	id: String,
	ip: Option<String>,
	port: Option<u16>,
	user_name: Option<String>,
	password: Option<String>,
}

impl SwServerMqtt {
	pub async fn run(&self, system: &mut System) -> Result<(), Error> {
		let (sender_id, rx, tx) = system.get_channel();
		let debug = system.config.debug;
		let host = match &self.ip { Some(ip) => ip.clone(), None => system.config.ip.clone() };
		let port = match self.port { Some(p) => p, None => 1883 };
		let addr = format!("{}:{}", host, port);
		let user_name = self.user_name.clone();
		let password = self.password.clone();
		let devices_by_client_id: HashMap<String, Vec<(usize, Arc<SwDeviceInstance>)>> = HashMap::new();
		let devices_by_client_id_am = Arc::new(Mutex::new(devices_by_client_id));
		let devices_by_client_id_amr = devices_by_client_id_am.clone();
		let subscribers_by_attr: HashMap<String, (SwConn, HashSet<String>)> = HashMap::new();	//client_id -> (conn, attrs)
		let subscribers_by_attr_am = Arc::new(Mutex::new(subscribers_by_attr));
		let subscribers_by_attr_amr = subscribers_by_attr_am.clone();
		let vals = Arc::new(Mutex::new(HashMap::<String, SwValue>::new()));
		let vals_am = vals.clone();
		let txr = tx.clone();

		//observe all
		tx.send(SwMessage::ObserveAttr{ sender_id, name: "#".to_string() }).await?;
		tx.send(SwMessage::Initialized{ sender_id, msg: format!("Listening for MQTT connections on addr {}", &addr) }).await?;
		tx.send(SwMessage::RegisterServer{ sender_id, server_id: self.id.clone(), server_data: SwServerData::Mqtt{ host: host.clone(), port, user_name: user_name.clone(), password: password.clone() } }).await?;

		task::spawn(async move {
			loop {
				//bind in loop in case of possible interface reconnect, etc.
				//in that case incoming iterator can return None or Err
				match TcpListener::bind(&addr).await {
					Ok(listener) => {
						loop {
							let mut incoming = listener.incoming();

							match incoming.next().await {
								Some(Ok(mut stream)) => {
									let devices_by_client_id_am = devices_by_client_id_amr.clone();
									let devices_by_client_id_amc = devices_by_client_id_amr.clone();
									let subscribers_by_attr_am = subscribers_by_attr_amr.clone();
									let subscribers_by_attr_amc = subscribers_by_attr_am.clone();
									let tx = txr.clone();
									let user_name_c = user_name.clone();
									let password_c = password.clone();
									let vals_c = vals_am.clone();
									let mut protocol_level = mqtt_packet::MAX_PROTOCOL_LEVEL;

									task::spawn(async move {
										if let Ok(p) = MqttPacket::async_read_frame(&mut stream, &protocol_level).await {
											if let MqttPacket::Connect(cp) = &p {

												let auth_ok = (user_name_c == None || cp.user_name == user_name_c) && (password_c == None || cp.password == password_c);
												let return_code = if auth_ok { ConnackReturnCode::Accepted } else { ConnackReturnCode::NotAuthorized };

												protocol_level = cp.protocol_level;

												_ = MqttPacket::Connack(ConnackProps {
													return_code,
													..Default::default()
												}).async_write_frame(&mut stream, &protocol_level).await;

												if auth_ok {
													let txr = &tx;
													let mut v = Vec::new();

													{
														//block due to release lock
														if let Some(devs) = devices_by_client_id_am.lock().await.get(&cp.client_identifier) {
															for (service_id, _) in devs {
																v.push(*service_id);
															}
														}
													}

													let is_outer_conn = v.len() == 0;

													if !is_outer_conn {
														for service_id in v {
															//send current connection to write thread
															_ = tx.send(SwMessage::SendTo{ sender_id, receiver_id: service_id, msg: Box::new(SwMessage::DevConn{ sender_id, conn: SwConn::Mqtt{ stream: stream.clone(), protocol_level }} ) }).await;
														}
													}

													let client_identifier = cp.client_identifier.clone();
													let stream_c = stream.clone();	//for shutdown

													#[allow(unreachable_code)]
													if let Err(e) = async move {
														loop {
															let p = MqttPacket::async_read_frame(&mut stream, &protocol_level).await?;

															match p {
																MqttPacket::Disconnect(_) => {
																	//removing disconnected clients in DevAttr section

																	return Ok(());
																},
																MqttPacket::Subscribe(sp) => {
																	let mut v = Vec::new();

																	let mut topic_attrs = Vec::with_capacity(sp.topics.len());

																	for t in &sp.topics {
																		v.push(SubackRet::Success(t.req_qos));

																		//compute attr from topic

																		if t.topic_name == "#" {
																			topic_attrs.push("#".to_string());
																		} else {
																			let topic_elems: Vec<&str> = t.topic_name.split('/').collect();

																			if topic_elems.len() == 4 && topic_elems[0] == "devices" && topic_elems[2] == "actual" {
																				topic_attrs.push(format!("{}.{}", topic_elems[1], topic_elems[3]));
																			}
																		}
																	}

																	if is_outer_conn {
																		//remember outer subscribers

																		{
																			let mut subscribers_by_attr = subscribers_by_attr_am.lock().await;

																			for attr in &topic_attrs {

																				//remember attr name for connection
																				println!("outer subscriber: {}: {}", client_identifier, attr);

																				if !subscribers_by_attr.contains_key(&client_identifier) {
																					subscribers_by_attr.insert(client_identifier.clone(), (SwConn::Mqtt{ stream: stream.clone(), protocol_level }, HashSet::new()));
																				}

																				if let Some((_, ref mut attrs)) = subscribers_by_attr.get_mut(&client_identifier) {
																					attrs.insert(attr.clone());
																				}
																			}
																		}
																	}

																	let mut sap = SubackProps{
																		packet_identifier: sp.packet_identifier,
																		topics: v,
																		..Default::default()
																	};

																	if protocol_level == 5 {
																		let mut properties = Vec::new();

																		//some clients require at least one property (MQTTX)
																		properties.push(MqttProperty::ReasonString("Welcome".to_string()));

																		sap.properties = Some(properties);
																	}

																	_ = MqttPacket::Suback(sap).async_write_frame(&mut stream, &protocol_level).await?;

																	//send current values

																	if is_outer_conn {
																		let mut to_send = Vec::new();

																		{
																			let vals = vals_c.lock().await;

																			for attr in topic_attrs {
																				if attr == "#" {
																					for (attr, val) in vals.iter() {
																						to_send.push((attr.to_string(), val.clone()));
																					}
																				}
																				else {
																					if let Some(val) = vals.get(&attr) {
																						to_send.push((attr, val.clone()));
																					}
																				}
																			}
																		}

																		for (name, value) in to_send {
																			let a: Vec<&str> = name.split('.').collect();

																			if a.len() == 2 {
																				_ = MqttPacket::Publish(PublishProps {
																					topic_name: format!("devices/{}/actual/{}", a[0], a[1]),
																					payload: value.to_string().into_bytes(),
																					..Default::default()
																				}).async_write_frame(&mut stream, &protocol_level).await?;
																			}
																		}
																	}
																},
																MqttPacket::Unsubscribe(up) => {
																	MqttPacket::Unsuback(UnsubackProps{
																		packet_identifier: up.packet_identifier,
																		..Default::default()
																	}).async_write_frame(&mut stream, &protocol_level).await?;
																},
																MqttPacket::Publish(pp) => {

																	if debug > Some(0) {
																		println!("Received publish packet: {:?}", pp);
																	}

																	match pp.qos {
																		0 => {
																			//do not send response
																		},
																		1 => MqttPacket::Puback(PubackProps{
																				packet_identifier: pp.packet_identifier,
																				..Default::default()
																			}).async_write_frame(&mut stream, &protocol_level).await?,
																		_ => {
																			//other todo
																		}
																	}

																	let mut service_info = None;

																	//cached payload reused in selector and send data
																	let mut dev_data = None;
																	let mut d: Option<Vec<(usize, Arc<SwDeviceInstance>)>> = None;

																	{
																		//block due to lock
																		//search by selector
																		if let Some(devs) = devices_by_client_id_am.lock().await.get(&client_identifier) {
																			d = Some(devs.iter().map(|(service_id, dev)| (*service_id, dev.clone())).collect());
																		}
																	}

																	if let Some(devs) = d {
																		for (service_id, dev) in devs {
																			let d = SwData {
																				val: SwDataValue::Raw(pp.payload.clone()),
																				src: SwDataSource::Mqtt { topic: pp.topic_name.clone() }
																			};

																			dev_data = Some(d);

																			let mut ctx = SwEvalCtx{ data: dev_data, dev: Some((dev.clone(), service_id)), ..Default::default() };

																			if let SwDataEventType::InMqtt(m) = &dev.dev_group.data_event {
																				if let Some(selector) = &m.selector {
																					match selector.eval(&mut ctx) {
																						Ok(SwValue::Bool(true)) => {
																							service_info = Some((service_id, dev));
																							dev_data = ctx.data;
																							break;
																						},
																						Err(e) => {
																							println!("Error in selector eval: {}, selector: {:?}", e, selector);
																						},
																						_ => {}
																					}
																				}
																			}

																			dev_data = ctx.data;
																		}
																	}
																	else {
																		//outer client
																		let topic_elems: Vec<&str> = pp.topic_name.split('/').collect();

																		if topic_elems.len() == 4 && topic_elems[0] == "devices" && topic_elems[2] == "store" {
																			if let Ok(s) = std::str::from_utf8(&pp.payload) {

																				//todo - swValue as string
																				txr.send(SwMessage::DevAttr{ sender_id: 0, name: format!("{}.{}", topic_elems[1], topic_elems[3]), value: SwValue::from_str(s)? }).await?;
																			}
																		}
																	}

																	if let Some((service_id, _)) = &service_info {
																		//send info to dev
																		txr.send(SwMessage::SendTo{ 
																			sender_id,
																			receiver_id: *service_id,
																			msg: Box::new(SwMessage::DevData {
																				sender_id,
																				data: if let Some(sw_data) = dev_data { sw_data } else {
																					SwData {
																						val: SwDataValue::Raw(pp.payload),
																						src: SwDataSource::Mqtt { topic: pp.topic_name }
																					}
																				}
																			})
																		}).await?;
																	}
																},
																MqttPacket::Pingreq => {
																	MqttPacket::Pingresp.async_write_frame(&mut stream, &protocol_level).await?;
																},
																_ => {
																}
															}
														}

														Ok::<(), Error>(())
													}.await {
														println!("Error read {}: {}, disconnect", cp.client_identifier, e);
													}
													else {
														println!("Client disconnected: {}", cp.client_identifier);
													}

													//inform devices
													_ = stream_c.shutdown(Shutdown::Both);
												}
												else {
													println!("Client not authorized: {}", cp.client_identifier);
												}

												//disconnect devices from stream

												let mut v = Vec::new();

												{
													//block due to release lock
													if let Some(devs) = devices_by_client_id_amc.lock().await.get(&cp.client_identifier) {
														for (service_id, _) in devs {
															v.push(*service_id);
														}
													}
												}

												for service_id in v {
													//send empty connection to write thread
													_ = tx.send(SwMessage::SendTo{ sender_id, receiver_id: service_id, msg: Box::new(SwMessage::DevConn{ sender_id, conn: SwConn::None } ) }).await;
												}

												{
													//delete outer subscribers
													let mut subscribers_by_attr = subscribers_by_attr_amc.lock().await;
													subscribers_by_attr.remove(&cp.client_identifier);
												}
											}
										}
									});
								},
								Some(Err(e)) => {
									println!("Error in incoming mqtt listener: {}", e);
									break;
								},
								None => {
									println!("No incoming mqtt connection");
									break;
								}
							}
						}
					},
					Err(e) => {
						println!("Error in bind on addr {}: {:?}", addr, e);
						task::sleep(Duration::from_secs(5)).await;
					}
				}
			}
		});

		//receive mqtt commands

		task::spawn(async move {
			loop {
				if let Err(e) = async {
					let received = rx.recv().await?;

					match received {
						SwMessage::RegisterService{ sender_id, server_id: _, service_type } => {
							if let SwServiceType::MqttDev{ dev } = service_type {
								println!("Register MQTT service: {}, {}", sender_id, dev.id);

								//remember device
								if let SwDataEventType::InMqtt(m) = &dev.dev_group.data_event {
									if let Some(value) = &m.client_id {
										let mut ctx = SwEvalCtx{ dev: Some((dev.clone(), sender_id)), ..Default::default() };

										if let Ok(SwValue::String(client_id)) = value.eval(&mut ctx) {
											devices_by_client_id_am.lock().await.entry(client_id).or_insert_with(Vec::new).push((sender_id, dev));
										}
									}
								}
							}
						},
						SwMessage::DevAttr{ sender_id: _, name, value } => {
							//inform subscribers

							//clients to inform
							let mut cti = Vec::new();

							{
								let mut subscribers_by_attr = subscribers_by_attr_am.lock().await;

								for n in vec![name.as_str(), "#"] {
									for (client_id, (ref mut conn, ref mut attrs)) in subscribers_by_attr.iter_mut() {
										if attrs.contains(n) {
											let a: Vec<&str> = name.split('.').collect();

											if a.len() == 2 {
												let p = MqttPacket::Publish(PublishProps {
													topic_name: format!("devices/{}/actual/{}", a[0], a[1]),
													payload: value.to_string().into_bytes(),
													..Default::default()
												});

												cti.push((client_id.clone(), p, std::mem::take(conn)));
											}
										}
									}
								}
							}

							for (client_id, p, ref mut conn) in cti.iter_mut() {
								if let SwConn::Mqtt{ ref mut stream, protocol_level } = conn {
									if let Err(e) = p.async_write_frame(stream, protocol_level).await {
										println!("Error sending mqtt packet for subscriber {}: {}", client_id, e);

										//to delete from map
										*conn = SwConn::None;
									}
								}
							}

							{
								let mut subscribers_by_attr = subscribers_by_attr_am.lock().await;

								//give back connections
								for (client_id, _, v_conn) in cti.into_iter() {
									if let Some((ref mut conn, _)) = subscribers_by_attr.get_mut(&client_id) {
										if let SwConn::None = conn {
											_ = std::mem::replace(conn, v_conn);
										}
									}
								}

								//remove clients without valid connection
								subscribers_by_attr.retain(|_, (conn, _)| {
									if let SwConn::Mqtt{ stream: _, protocol_level: _ } = conn {
										return true;
									}

									false
								});
							}

							{
								//for later incoming outer clients

								let mut vals = vals.lock().await;
								vals.insert(name, value);
							}
						}
						_ => {}
					}

					Ok::<(), Error>(())
				}.await {
					println!("Error in mqtt server loop: {:?}", e);
				}
			}
		});

		Ok(())
	}
}