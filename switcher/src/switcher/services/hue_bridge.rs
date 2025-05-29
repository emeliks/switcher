use serde::{ Serialize, Deserialize };
use std::{ sync::Arc, net::{ Ipv4Addr }, collections::HashMap, ops::Deref, time::SystemTime, str::FromStr };
use crate::compat::{ task, net::{ UdpSocket }, io::{ Cursor }, sync::{ Mutex } };
use net_services::{ http::{ HttpHeaders, async_http_connection } };
use crate::{ switcher::{ SwMessage, SwServiceType, SwServiceData, SwServerData, System, sw_device::{ self, SwValue, Error, RGBW } } };

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HueBridge {
	//alexa doesn't send /api request on port other than 80!
	server_id: Option<String>,
	path: Option<String>,
	ip: Option<String>,
	#[serde(default = "HueBridge::default_uuid")]
	uuid: String,
	lights: HashMap<String, HueLight>
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SimpleLightState {
	on: bool,
	reachable: bool
}

#[derive(Serialize, Deserialize, Clone, Debug)]
enum ColorMode {
	#[serde(rename = "ct")]
	Ct,
	#[serde(rename = "hs")]
	Hs,
	#[serde(rename = "xy")]
	Xy
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ExtendedLightState {
	on: bool,
	reachable: bool,
	bri: u8,
	hue: u16,
	sat: u8,
	xy: (f32, f32),
	ct: u16,
	#[serde(skip_serializing)]
	rgbw: RGBW,
	alert: String,
	effect: String,
	colormode: ColorMode,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SimpleLight{
	name: String,
	#[serde(skip_serializing)]
	attr: String,
	#[serde(default = "HueBridge::default_pointsymbol")]
	pointsymbol: HashMap<String, String>,
	#[serde(default = "HueBridge::default_manufacturer")]
	manufacturername: String,
	#[serde(default = "HueBridge::default_modelid")]
	modelid: String,
	#[serde(default = "HueBridge::default_swversion")]
	swversion: String,
	#[serde(default = "HueBridge::default_uniqueid")]
	uniqueid: String,
	#[serde(default = "HueBridge::default_simplestate")]
	state: SimpleLightState,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ExtendedLight {
	name: String,
	#[serde(skip_serializing)]
	power_attr: String,
	#[serde(skip_serializing)]
	rgbw_attr: String,
	#[serde(default = "HueBridge::default_pointsymbol")]
	pointsymbol: HashMap<String, String>,
	#[serde(default = "HueBridge::default_manufacturer")]
	manufacturername: String,
	#[serde(default = "HueBridge::default_modelid")]
	modelid: String,
	#[serde(default = "HueBridge::default_swversion")]
	swversion: String,
	#[serde(default = "HueBridge::default_uniqueid")]
	uniqueid: String,
	#[serde(default = "HueBridge::default_extendedstate")]
	state: ExtendedLightState,
}

impl ExtendedLightState {
	fn rgb_to_hsb(rgbw: &RGBW) -> (f32, f32, f32) {
		let mut hue: f32;
		let saturation: f32;
		let brightness: f32;

		let r;
		let g;
		let b;

		if rgbw.w != 0 {
			r = rgbw.w;
			g = rgbw.w;
			b = rgbw.w;
		}
		else {
			r = rgbw.r;
			g = rgbw.g;
			b = rgbw.b;
		}

		let mut cmax = if r > g { r } else { g };
		if b > cmax {
			cmax = b;
		}

		let mut cmin = if r < g { r } else { g };
		if b < cmin {
			cmin = b;
		}

		brightness = cmax as f32 / 255.0;

		if cmax != 0 {
			saturation = (cmax - cmin) as f32 / cmax as f32;
		}
		else {
			saturation = 0.0;
		}

		if saturation == 0.0 {
			hue = 0.0;
		}
		else {
			let redc = (cmax - r) as f32 / (cmax - cmin) as f32;
			let greenc = (cmax - g) as f32 / (cmax - cmin) as f32;
			let bluec = (cmax - b) as f32 / (cmax - cmin) as f32;

			if r == cmax {
				hue = bluec - greenc;
			}
			else {
				if g == cmax {
					hue = 2.0 + redc - bluec;
				}
				else {
					hue = 4.0 + greenc - redc;
				}
			}

			hue = hue / 6.0;

			if hue < 0.0 {
				hue = hue + 1.0;
			}
		}

		(hue, saturation, brightness)
	}

	fn hsb_to_rgb(hue: f32, saturation: f32, brightness: f32) -> RGBW {
		let mut r: u8 = 0;
		let mut g: u8 = 0;
		let mut b: u8 = 0;
		let mut w: u8 = 0;

		if saturation == 0.0 {
			w = (brightness * 255.0 + 0.5) as u8;
		}
		else {
			let h = (hue - hue.floor()) * 6.0;
			let f = h - h.floor();
			let p = brightness * (1.0 - saturation);
			let q = brightness * (1.0 - saturation * f);
			let t = brightness * (1.0 - (saturation * (1.0 - f)));

			match h as u8 {
				0 => {
					r = (brightness * 255.0 + 0.5) as u8;
					g = (t * 255.0 + 0.5) as u8;
					b = (p * 255.0 + 0.5) as u8;
				},
				1 => {
					r = (q * 255.0 + 0.5) as u8;
					g = (brightness * 255.0 + 0.5) as u8;
					b = (p * 255.0 + 0.5) as u8;
				},
				2 => {
					r = (p * 255.0 + 0.5) as u8;
					g = (brightness * 255.0 + 0.5) as u8;
					b = (t * 255.0 + 0.5) as u8;
				},
				3 => {
					r = (p * 255.0 + 0.5) as u8;
					g = (q * 255.0 + 0.5) as u8;
					b = (brightness * 255.0 + 0.5) as u8;
				},
				4 => {
					r = (t * 255.0 + 0.5) as u8;
					g = (p * 255.0 + 0.5) as u8;
					b = (brightness * 255.0 + 0.5) as u8;
				},
				5 => {
					r = (brightness * 255.0 + 0.5) as u8;
					g = (p * 255.0 + 0.5) as u8;
					b = (q * 255.0 + 0.5) as u8;
				},
				_ => {}
			}
		}

		RGBW{r, g, b, w}
	}

	fn update_from_color(&mut self) {
		match self.colormode {
			ColorMode::Hs => {
				//sets rgbw from hsb

				let hue = self.hue as f32 / 65535.0;
				let saturation = self.sat as f32 / 255.0;
				let brightness = self.bri as f32 / 255.0;

				self.rgbw = Self::hsb_to_rgb(hue, saturation, brightness);
			},
			ColorMode::Ct => {
				self.rgbw = RGBW{r: 0, g: 0, b: 0, w: self.bri};
			},
			_ => {}
		}
	}

	fn update_from_rgbw(&mut self) {
		if self.rgbw.w == 0 {
			let (hue, sat, bri) = Self::rgb_to_hsb(&self.rgbw);

			self.hue = (hue * 65535.0 + 0.5) as u16;
			self.sat = (sat * 255.0 + 0.5) as u8;
			self.bri = (bri * 255.0 + 0.5) as u8;

			self.colormode = ColorMode::Hs;
		}
		else {
			self.bri = self.rgbw.w;
			self.colormode = ColorMode::Ct;
		}
	}
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
enum HueLight {
	#[serde(rename = "On/off light")]
	Simple(SimpleLight),
	#[serde(rename = "Extended color light")]
	Extended(ExtendedLight)
}

impl HueBridge {
	fn default_uuid() -> String {
		"30d64012-d519-11ed-afa1-0242ac120002".to_string()
	}

	fn default_pointsymbol() -> HashMap<String, String> {
		HashMap::new()
	}

	fn default_manufacturer() -> String {
		"OSRAM".to_string()
	}

	fn default_modelid() -> String {
		"Plug 01".to_string()
	}

	fn default_swversion() -> String {
		"v1.04.12".to_string()
	}

	fn default_simplestate() -> SimpleLightState {
		SimpleLightState {
			on: false,
			reachable: true
		}
	}

	fn default_extendedstate() -> ExtendedLightState {
		ExtendedLightState {
			on: false,
			reachable: true,
			bri: 0,
			hue: 0,
			sat: 0,
			xy: (0.0, 0.0),
			ct: 150,	// = 1000000/K
			rgbw: Default::default(),
			alert: "none".to_string(),
			effect: "none".to_string(),
			colormode: ColorMode::Hs,
		}
	}

	fn default_uniqueid() -> String {
		if let Ok(x) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
			let mut x = x.as_nanos();
			let mut uid = "".to_string();

			for i in 0..9 {
				if uid.len() != 0 {
					uid.push(if i == 8 { '-' } else { ':' });
				}

				uid.push_str(&format!("{:02x}", x as u8));
				x = x >> 8;
			}

			uid
		}
		else {
			"00:11:22:33:44:55:66:77-88".to_string()
		}
	}
}

impl HueBridge {
	pub async fn run(&self, system: &mut System) -> Result<(), Error> {

		let (dev_id, rx, tx) = system.get_channel();

		let path = if let Some(path) = &self.path { path.clone() } else { "/".to_string() };
		let path_len = path.len();

		tx.send(SwMessage::RegisterService{
			sender_id: dev_id,
			server_id: self.server_id.clone(),
			service_type: SwServiceType::Http{ path: path.clone(), name: "Hue bridge server".to_string() }
		}).await?;

		let config_uuid = self.uuid.clone();
		let tx_clone = tx.clone();

		//observe values from devices

		let mut dev_light_map = HashMap::new();

		for (k, v) in &self.lights {
			match v {
				HueLight::Simple(l) => {
					dev_light_map.insert(l.attr.clone(), k.clone());
					tx.send(SwMessage::ObserveAttr{sender_id: dev_id, name: l.attr.clone()}).await?;
				},
				HueLight::Extended(e) => {
					dev_light_map.insert(e.power_attr.clone(), k.clone());
					dev_light_map.insert(e.rgbw_attr.clone(), k.clone());

					tx.send(SwMessage::ObserveAttr{sender_id: dev_id, name: e.power_attr.clone()}).await?;
					tx.send(SwMessage::ObserveAttr{sender_id: dev_id, name: e.rgbw_attr.clone()}).await?;
				}
			}
		}

		let lights = self.lights.clone();
		let lights = Arc::new(Mutex::new(lights));

		let lights_clone = lights.clone();
		let debug = system.config.debug;
		let ip = match &self.ip { Some(ip) => ip.clone(), None => system.config.ip.clone() };

		#[allow(unreachable_code)]
		task::spawn(async move {
			let mut addr = None;

			loop {
				if let Err(e) = async {
					let received = rx.recv().await?;

					match received {
						SwMessage::DevAttr{sender_id: _, name, value} => {
							if let Some(v) = &dev_light_map.get(&name) {
								let mut lights = lights_clone.lock().await;
								if let Some(l) = lights.get_mut(*v) {
									match l {
										HueLight::Simple(s) => {
											s.state.on = if let SwValue::Bool(true) = value { true } else { false };
										},
										HueLight::Extended(e) => {
											match value {
												SwValue::Bool(true) => { e.state.on = true; },
												SwValue::Bool(false) => { e.state.on = false; },
												SwValue::String(s) => {
													let rgbw: RGBW = RGBW::from_str(&s)?;
													if e.state.rgbw != rgbw {
														e.state.rgbw = rgbw;
														e.state.update_from_rgbw();
													}
												},
												_ => {}
											}
										},
									}
								}
							}
						},
						SwMessage::ServiceRegistered{ server_data, .. } => {
							if let SwServerData::Http{ host, port } = server_data {
								//udp discover service

								//this config works - bind on 0.0.0.0 and join on host ip
								let socket = UdpSocket::bind(format!("{}:1900", ip)).await?;
								socket.join_multicast_v4(Ipv4Addr::new(239, 255, 255, 250), host.parse::<Ipv4Addr>().unwrap())?;

								tx.send(SwMessage::Initialized{sender_id: dev_id, msg: format!("Listening Hue Discover service on addr {}:1900", ip)}).await?;

								let s_addr = format!("{}:{}", host, port);
								addr = Some((host, port));

								#[allow(unreachable_code)]
								task::spawn(async move {
									let mut buffer = [0; 500];

									loop {
										if let Err(e) = async {
											if let Ok((amt, src)) = socket.recv_from(&mut buffer).await {
												if let Ok(c) = async_http_connection::incoming(&mut Cursor::new(&mut buffer[0..amt])).await {
													if let Some(s) = c.req.get_header("MAN") {
														if s == "\"ssdp:discover\"" {

															if debug > Some(1) {
																println!("Discover request from: {}", &src);
															}

															let mut resp = HttpHeaders::new();
															resp.header("CACHE-CONTROL", "max-age=100");
															resp.header("EXT", "");
															resp.header("LOCATION", &format!("http://{}/description.xml", &s_addr));
															resp.header("OPT", "\"http://schemas.upnp.org/upnp/1/0/\"; ns=01");
															resp.header("ST", "upnp:rootdevice");
															resp.header("USN", "uuid:2fa00080-d000-11e1-9b23-001f80007bbe::upnp:rootdevice");

															let mut bc = Cursor::new(Vec::new());

															if let Err(e) = async_http_connection::write_http_response(&c.protocol, &resp, &mut bc, 200, "OK").await {
																eprintln!("error: {:?}", e);
															}

															let out_buf = bc.into_inner();

															if debug > Some(1) {
																println!("Discover response: {}", std::str::from_utf8(&out_buf).unwrap() );
															}

															if let Err(e) = socket.send_to(&out_buf, src).await {
																eprintln!("error: {:?}", e);
															}

															if debug > Some(1) {
																println!("Discover response to {}: {}", &src, &s_addr);
															}
														}
													}
												}
											}

											Ok::<(), sw_device::Error>(())
										}.await {
											println!("Error in hue discover loop: {:?}", e);
										}
									}
								});
							}
						},
						SwMessage::Serve{sender_id: _, data: SwServiceData::Http{ mut c, conn: mut stream } } => {
							if c.path.len() >= path_len {
								let p: Vec<&str> = c.path[path_len..].split('/').collect();	//skip own path
								let l = p.len();

								if debug > Some(1) {
									println!("Hue request {}", c.path);
								}

								match p[0] {
									"description.xml" => {
										if let Some((host, port)) = &addr {
											c.resp.header("Content-Type", "application/xml; charset=utf-8");
											async_http_connection::write_content(&mut c, &mut stream, format!("<root xmlns=\"urn:schemas-upnp-org:device-1-0\"><specVersion><major>1</major><minor>0</minor></specVersion><URLBase>http://{}:{}/</URLBase><device><deviceType>urn:schemas-upnp-org:device:Basic:1</deviceType><friendlyName>Philips hue ({})</friendlyName><manufacturer>Royal Philips Electronics</manufacturer><manufacturerURL>http://www.philips.com</manufacturerURL><modelDescription>Philips hue Personal Wireless Lighting</modelDescription><modelName>Philips hue bridge 2015</modelName><modelNumber>BSB002</modelNumber><modelURL>http://www.meethue.com</modelURL><serialNumber>9290002265073</serialNumber><UDN>uuid:{}</UDN></device></root>", &host, &port, &host, &config_uuid).as_bytes()).await?;
										}
									},
									"api" => {
										match l {
											1 => {
												if c.method == "POST" {
												}

												c.resp.header("Content-Type", "application/json");
												async_http_connection::write_content(&mut c, &mut stream, b"[{\"success\":{\"username\":\"switcher\"}}]").await?;
											},
											3 => {
												if p[2] == "lights" {
													c.resp.header("Content-Type", "application/json");
													let mut s = "".to_string();

													{
														let lights = lights_clone.lock().await;
														s += &serde_json::to_string(&lights.deref())?;
													}

													async_http_connection::write_content(&mut c, &mut stream, s.as_bytes()).await?;
												}
											},
											4 => {
												let id = &p[3];
												if p[2] == "lights" {
													let mut resp = "".to_string();

													{
														let lights = lights_clone.lock().await;
														resp += &serde_json::to_string(&lights.get(&id.to_string()))?;
													}

													c.resp.header("Content-Type", "application/json");
													async_http_connection::write_content(&mut c, &mut stream, resp.as_bytes()).await?;
												}
											},
											5 => {
												let id = &p[3].to_string();
												if p[2] == "lights" && p[4] == "state" {
													let s = async_http_connection::get_content_str(&c.req, &mut stream).await?;
													let v: serde_json::Value = serde_json::from_str(&s)?;

													if debug > Some(0) {
														println!("Hue request {}: {}", c.path, s);
													}

													let mut changed_value = None;
													let mut ojr = None;

													{
														let mut lights = lights_clone.lock().await;

														if let Some(hl) = lights.get_mut(id) {
															let mut jr = Vec::new();

															if let serde_json::Value::Object(m) = v {
																if let Some(serde_json::Value::Bool(on)) = m.get("on") {
																	let attr = match hl {
																		HueLight::Simple(l) => { l.state.on = *on; &l.attr },
																		HueLight::Extended(e) => { e.state.on = *on; &e.power_attr }
																	};

																	changed_value = Some((attr.clone(), if *on { SwValue::Bool(true) } else { SwValue::Bool(false) }));
																	jr.push(("on", serde_json::Value::Bool(*on)));
																}

																if let HueLight::Extended(e) = hl {
																	let mut modified = false;

																	for k in ["bri", "hue", "sat", "ct"] {
																		if let Some(mv) = m.get(k) {
																			modified = true;

																			if let serde_json::Value::Number(n) = mv {
																				jr.push((k, mv.clone()));

																				match k {
																					"bri" => { if let Some(n) = n.as_u64() { e.state.bri = n as u8 } },
																					"hue" => { if let Some(n) = n.as_u64() { e.state.colormode = ColorMode::Hs; e.state.hue = n as u16 } },
																					"sat" => { if let Some(n) = n.as_u64() { e.state.colormode = ColorMode::Hs; e.state.sat = n as u8 } },
																					"ct" => { if let Some(n) = n.as_u64() { e.state.colormode = ColorMode::Ct; e.state.ct = n as u16 } },
																					//todo xy
																					_ => {}
																				};
																			}
																		}
																	}

																	if modified {
																		e.state.update_from_color();
																		changed_value = Some((e.rgbw_attr.clone(), SwValue::String(e.state.rgbw.to_string())));
																	}
																}
															}

															ojr = Some(jr);
														}
													}

													if let Some((name, value)) = changed_value {
														tx_clone.send(SwMessage::DevAttr{ sender_id: 0, name, value }).await?;
													}

													let mut s = "".to_string();

													let pref = format!("/lights/{}/state/", id);

													s.push_str("[");

													if let Some(jr) = ojr {
														for (i, (k, v)) in jr.iter().enumerate() {
															if i > 0 {
																s.push_str(", ");
															}
															s.push_str("{\"success\": {");
															s.push_str(&format!("\"{}{}\": {}", pref, k, v));
															s.push_str("}}");
														}
													}

													s.push_str("]");

													c.resp.header("Content-Type", "application/json");
													async_http_connection::write_content(&mut c, &mut stream, s.as_bytes()).await?;

													if debug > Some(0) {
														println!("Hue response {}", s);
													}
												}
											},
											_ => {
											}
										}
									},
									_ => {
									}
								}
							}
						},
						_ => {
						}
					}

					Ok::<(), sw_device::Error>(())
				}.await {
					println!("Error in hue loop: {:?}", e);
				}
			}

		});

		Ok(())
	}
}
