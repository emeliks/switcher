use serde::{ Deserialize };
use crate::compat::{ sync::{ Mutex }, net::{ TcpListener }, task };
use std::{ sync::Arc, time::Duration };
use futures::stream::StreamExt;
use net_services::{ http::{ async_http_connection } };
use crate::{ switcher::{ SwServerData, SwServiceType, SwMessage, System, SwServiceData, sw_device::{ Error } } };

#[derive(Deserialize, Debug)]
pub struct SwServerHttp {
	id: String,
	ip: Option<String>,
	port: Option<u16>
}

impl SwServerHttp {
	pub async fn run(&self, system: &mut System) -> Result<(), Error> {
		let (sender_id, rx, tx) = system.get_channel();

		let host = match &self.ip { Some(ip) => ip.clone(), None => system.config.ip.clone() };
		let port = match self.port { Some(p) => p, None => 80 };
		let addr = format!("{}:{}", host, port);
		let devices_by_path: Vec<(String, usize)> = Vec::new();
		let devices_by_path_am = Arc::new(Mutex::new(devices_by_path));
		let devices_by_path_amr = devices_by_path_am.clone();
		let txr = tx.clone();

		tx.send(SwMessage::Initialized{ sender_id, msg: format!("Listening for HTTP connections on addr {}", &addr) }).await?;
		tx.send(SwMessage::RegisterServer{ sender_id, server_id: self.id.clone(), server_data: SwServerData::Http{ host: host.clone(), port } }).await?;

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
									match async_http_connection::incoming(&mut stream).await {
										Ok(c) => {
											if c.path.len() > 0 {
												let mut s_id = None;

												{
													for (path, service_id) in devices_by_path_amr.lock().await.iter() {
														if c.path.starts_with(path) {
															s_id = Some(*service_id);
															break;
														}
													}
												}

												if let Some(service_id) = s_id {
													if let Err(e) = txr.send(
														SwMessage::SendTo{
															sender_id,
															receiver_id: service_id,
															msg: Box::new(
																SwMessage::Serve{
																	sender_id,
																	data: SwServiceData::Http{ c, conn: stream.clone() }
																}
															)
														}
													).await {
														println!("Error sending response: {}", e);
													}
												}
											}
										},
										Err(e) => {
											println!("Error in http request: {}", e);
										}
									}
								},
								Some(Err(e)) => {
									println!("Error in incoming http listener: {}", e);
									break;
								},
								None => {
									println!("No incoming http connection");
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

		task::spawn(async move {
			loop {
				if let Err(e) = async {
					let received = rx.recv().await?;

					match received {
						SwMessage::RegisterService{ sender_id, server_id: _, service_type } => {
							if let SwServiceType::Http{ path, name } = service_type {
								println!("Register HTTP service: {} on {}", name, path);

								//remember device
								devices_by_path_am.lock().await.push((path, sender_id));
							}
						},
						_ => {}
					}

					Ok::<(), Error>(())
				}.await {
					println!("Error in http server loop: {:?}", e);
				}
			}
		});

		Ok(())
	}
}