use serde::{ Deserialize };
use net_services::{ http::{ HttpHeaders, async_http_connection, ReqOrResp } };
use crate::{ switcher::{ SwMessage, System, sw_device::{ Error } } };
use core::time::Duration;
use std::net::Ipv4Addr;
use crate::compat::{ task, net::{ UdpSocket }, io::{ Cursor } };

#[derive(Deserialize, Debug)]
pub struct DiscoverUpnp {
}

impl DiscoverUpnp {
	pub fn new() -> Self {
		DiscoverUpnp{
		}
	}

	pub async fn run(&self, system: &mut System) -> Result<(), Error> {
		let (dev_id, _, tx) = system.get_channel();

		//this config works - bind on 0.0.0.0 and join on host ip
		let socket = UdpSocket::bind(format!("{}:1900", "0.0.0.0")).await?;

		socket.join_multicast_v4(Ipv4Addr::new(239, 255, 255, 250), system.config.ip.parse::<Ipv4Addr>().unwrap())?;

		tx.send(SwMessage::Initialized{sender_id: dev_id, msg: format!("Discover upnp devices service on addr {}:1900", system.config.ip)}).await?;


		#[allow(unreachable_code)]
		task::spawn(async move {
			let mut buffer = [0; 500];

			let mut req = HttpHeaders::new();
			req.header("HOST", "239.255.255.250:1900");
			req.header("MAN", "\"ssdp:discover\"");
			req.header("MX", "10");
			req.header("ST", "ssdp:all");

			let mut c = Cursor::new(Vec::new());
			async_http_connection::write_http_request("M-SEARCH", "*", "HTTP/1.1", &req, &mut c).await?;

			let out_buf = c.into_inner();
			let i = 0;

			loop {
				if i % 10 == 0 {
					_ = socket.send_to(&out_buf, "239.255.255.250:1900").await;
					task::sleep(Duration::from_millis(10000)).await;
				}

				if let Ok((amt, _src)) = socket.recv_from(&mut buffer).await {
					//possible are request NOTIFY or response to our request
					//serve both cases
					match async_http_connection::http_read_req_or_resp(&mut Cursor::new(&mut buffer[0..amt])).await {
						Ok(resp) => {
							println!("response: {:?}", resp);

							let h = match resp { ReqOrResp::Req(c) => c.req, ReqOrResp::Resp(r) => r.headers };

							println!("response: {:?}", h.get_header("location"));
						},
						Err(e) => {
							println!("response: error: {:?}", e);
						}
					}
				}
			}

			Ok::<(), Error>(())
		});

		Ok(())
	}
}
