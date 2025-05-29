use serde::{ Serialize, Deserialize };
use net_services::{ websocket, http::{ async_http_connection }, compat::AsyncWrite };
use crate::{ switcher::{ SwMessage, SwServiceData, SwServiceType, System, sw_device::{ Error } } };
use crate::compat::{ task, net::TcpStream, io::{ self }, fs::File };
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug)]
pub struct HttpSwitcher {
	server_id: Option<String>,
	path: Option<String>,
	dir: Option<String>,
	system: SwHttpSystem,
	mqtt_ip: Option<String>,
	mqtt_port: Option<u16>
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SwHttpSystem {
	id: String,
	pages: Vec<SwHttpPage>
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SwHttpPage {
	title: String,
	elements: Vec<SwHttpElement>
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum SwHttpCtrl {
	#[serde(rename = "switch")]
	Switch(SwHttpCtrlSwitch),
	#[serde(rename = "switch_ro")]
	SwitchRo(SwHttpCtrlSwitch),
	#[serde(rename = "rgb")]
	ColorRgb(SwHttpCtrlRgb),
	#[serde(rename = "button")]
	Button(SwHttpCtrlSwitch),
	#[serde(rename = "select")]
	Select(SwHttpCtrlSelect),
	#[serde(rename = "text")]
	Text(SwHttpCtrlText),
	#[serde(rename = "range")]
	Range(SwHttpCtrlRange)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SwHttpProps {
	#[serde(skip_serializing_if = "Option::is_none")]
	reachable: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	signal: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	battery: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SwHttpCtrlRange {
	#[serde(skip_serializing_if = "Option::is_none")]
	title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	role: Option<String>,
	attr: String,
	min: u16,
	max: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SwHttpCtrlRgb {
	#[serde(skip_serializing_if = "Option::is_none")]
	title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	role: Option<String>,
	attr: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SwHttpCtrlSwitch {
	#[serde(skip_serializing_if = "Option::is_none")]
	title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	role: Option<String>,
	attr: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SwHttpCtrlText {
	#[serde(skip_serializing_if = "Option::is_none")]
	title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	role: Option<String>,
	attr: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	round: Option<u16>,
	#[serde(skip_serializing_if = "Option::is_none")]
	unit: Option<String>
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SwHttpCtrlSelect {
	#[serde(skip_serializing_if = "Option::is_none")]
	title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	role: Option<String>,
	attr: String,
	value_map: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SwHttpElement{
	#[serde(skip_serializing_if = "Option::is_none")]
	title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	icon: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	display: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	props: Option<SwHttpProps>,
	controls: Vec<SwHttpCtrl>,
}

impl HttpSwitcher {
	pub async fn run(&self, system: &mut System) -> Result<(), Error> {

		let (dev_id, rx, tx) = system.get_channel();
		let path = if let Some(path) = &self.path { path.clone() } else { "/".to_string() };
		let dir = if let Some(dir) = &self.dir { dir.clone() } else { "./".to_string() };
		let mqtt_addr = format!("{}:{}", match &self.mqtt_ip { Some(mqtt_ip) => mqtt_ip, None => &system.config.ip }, match &self.mqtt_port { Some(mqtt_port) => *mqtt_port, None => 1883 });

		let path_len = path.len();
		let http_system = self.system.clone();

		tx.send(SwMessage::RegisterService{
			sender_id: dev_id,
			server_id: self.server_id.clone(),
			service_type: SwServiceType::Http{ path: path.clone(), name: format!("Switcher Server, mqtt to: {}", mqtt_addr) }
		}).await?;

		#[allow(unreachable_code)]
		task::spawn(async move {
			loop {
				if let Err(e) = async {
					let received = rx.recv().await?;

					match received {
						SwMessage::Serve{sender_id: _, data: SwServiceData::Http{ mut c, conn: mut stream } } => {
							if c.path.len() >= path_len {
								let p: Vec<&str> = c.path[path_len..].split('/').collect();	//skip first own path
								let pl = p.len();

								if pl == 1 {
									match p[0] {
										"index.html" => {
											c.resp.header("Content-Type", "text/html");
											async_http_connection::flush(&mut c, &mut stream).await?;
											io::copy(&mut File::open(format!("{}switcher_http/index.html", dir)).await?, &mut stream).await?;
										},
										"css.css" => {
											c.resp.header("Content-Type", "text/css");
											async_http_connection::flush(&mut c, &mut stream).await?;
											io::copy(&mut File::open(format!("{}switcher_http/css.css", dir)).await?, &mut stream).await?;
											stream.write_all(b"\n").await?;
											io::copy(&mut File::open(format!("{}switcher_http/swiper-bundle.min.css", dir)).await?, &mut stream).await?;
											stream.write_all(b"\n").await?;
										},
										"js.js" => {
											c.resp.header("Content-Type", "text/javascript;charset=utf-8");
											c.auto_content_length = false;
											async_http_connection::flush(&mut c, &mut stream).await?;
											io::copy(&mut File::open(format!("{}switcher_http/mqtt.min.js", dir)).await?, &mut stream).await?;
											stream.write_all(b"\n").await?;
											io::copy(&mut File::open(format!("{}switcher_http/swiper-bundle.min.js", dir)).await?, &mut stream).await?;
											stream.write_all(b"\n").await?;
											io::copy(&mut File::open(format!("{}switcher_http/js.js", dir)).await?, &mut stream).await?;
											stream.write_all(b"\n").await?;
											stream.write_all(b"var system=").await?;
											stream.write_all(serde_json::to_string(&http_system)?.as_bytes()).await?;
											stream.write_all(b";").await?;
										},
										"mqtt" => {
											websocket::async_write_handshake(&mut c, &mut stream).await?;
											let (a, b) = websocket::create_ws_connection(&mut stream, &mut TcpStream::connect(&mqtt_addr).await?).await?;

											task::spawn(a);
											task::spawn(b);
										},
										_ => {}
									}
								}
							}
						},
						_ => {}
					}

					Ok::<(), Error>(())
				}.await {
					println!("Error in http switcher service loop: {:?}", e);
				}
			}
		});

		Ok(())
	}
}
